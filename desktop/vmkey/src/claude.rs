//! `aiwalk-claude-vm [vault]`: Claude Code in Android's Linux Terminal, for Obsidian on the same phone. Each Claude runs
//! in a tmux session `exo-<name>`, as on a computer (`exo-claude` starts exo-phone); `GET /sessions` lists them and the
//! earlier conversations, `/session/end` and `/conversation/remove` are the Claude sessions window's buttons, and every
//! other request takes `session=<name>` (exo-phone when left out). `/logs` says `"sessions":true` and gives each
//! conversation's title (its last rename; a session's Claude is started with `-n <name>`), so the pet knows whose it is. This answers Obsidian on port 47172, which the Terminal
//! forwards to the phone. `POST /cite` types its body into Claude's prompt (not sent); `GET /logs` lists the vault's
//! conversation logs (and the vault's path here) and `GET /log?id=&from=` gives one from a byte offset, so the pet
//! follows what Claude edits. `/term/...` is that same tmux session as a terminal inside Obsidian, so Claude shows in
//! a pane there as it does on a computer, with no split screen. Typing into Claude is what a stolen token would be
//! used for, so the token is not in the vault, where any app with storage access reads it: it stays in this VM
//! (~/.config/aiwalk/claude-vm-token), and Obsidian gets it once, for a 6-digit code (`GET /pair?code=`).
//!
//! `aiwalk-claude-vm install` sets the VM up once: `exo-claude` in ~/.local/bin, and this program as a systemd user
//! service, so it runs from the VM's start and Obsidian finds it without anyone opening a tab for it.
//! `aiwalk-claude-vm pair` prints a new code.

// phonehttp.rs seals notes too, so it brings the key code along; this program never uses it
#[cfg(target_os = "linux")]
#[path = "../../src/fidokey.rs"]
#[allow(dead_code)]
mod fidokey;
#[cfg(target_os = "linux")]
#[path = "../../src/agefido.rs"]
#[allow(dead_code)]
mod agefido;
#[cfg(target_os = "linux")]
#[path = "../../src/phonehttp.rs"]
#[allow(dead_code)]
mod phonehttp;

#[cfg(target_os = "linux")]
mod vm {
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    const PORT: u16 = 47172;
    const VAULT: &str = "/mnt/shared/Documents/aIwalk/exo-book";
    const PREFIX: &str = "exo-";
    /// One read of a log at most: the pet asks again for the rest.
    const CHUNK: u64 = 4 << 20;
    /// The terminal output kept for Obsidian to catch up on; older bytes are dropped.
    const KEEP: usize = 1 << 20;

    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    static TOKEN: OnceLock<String> = OnceLock::new();

    fn home() -> PathBuf { PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/root".into())) }
    fn config(name: &str) -> PathBuf { home().join(".config/aiwalk").join(name) }
    /// Written readable by this user alone: the token and the pairing code are what a stranger would need.
    fn private(p: &Path, text: &str) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = std::fs::write(p, text);
        let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600));
    }
    /// Claude Code's folder for this vault: its path with every non-alphanumeric character turned into "-".
    fn logs_dir() -> PathBuf {
        let v: String = ROOT.get().unwrap().to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        home().join(".claude").join("projects").join(v)
    }
    /// The token, made once and kept in this VM only.
    fn token() -> String {
        let p = config("claude-vm-token");
        if let Ok(t) = std::fs::read_to_string(&p) { if t.trim().len() == 32 { return t.trim().into() } }
        let t = crate::phonehttp::token();
        private(&p, &t);
        t
    }
    fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }
    /// A new pairing code, good for ten minutes and once. Kept in a file, not in memory, so `aiwalk-claude-vm pair`
    /// can hand one to the copy running as a service.
    fn new_code() -> String {
        let code = format!("{:06}", u32::from_le_bytes(crate::phonehttp::token().as_bytes()[..4].try_into().unwrap()) % 1_000_000);
        private(&config("claude-vm-code"), &format!("{code} {}", now()));
        code
    }
    fn take_code(given: &str) -> bool {
        let p = config("claude-vm-code");
        let Ok(t) = std::fs::read_to_string(&p) else { return false };
        let mut it = t.split_whitespace();
        let (Some(code), Some(at)) = (it.next(), it.next().and_then(|s| s.parse::<u64>().ok())) else { return false };
        let ok = code == given && now().saturating_sub(at) < 600;
        if ok { let _ = std::fs::remove_file(&p); }
        ok
    }
    /// A log id as the plugin names it: `<uuid>` or `<uuid>/subagents/agent-<x>`, nothing that climbs out.
    fn log_path(id: &str) -> Option<PathBuf> {
        let ok = !id.is_empty() && !id.contains("..") && id.chars().all(|c| c.is_ascii_alphanumeric() || "-_/".contains(c));
        ok.then(|| logs_dir().join(format!("{id}.jsonl"))).filter(|p| p.is_file())
    }
    fn logs() -> Vec<(String, u64)> {
        let dir = logs_dir(); let mut out = Vec::new();
        let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let (p, name) = (e.path(), e.file_name().to_string_lossy().into_owned());
            if let Some(id) = name.strip_suffix(".jsonl") { out.push((id.to_string(), size(&p))); }
            else if p.is_dir() { for s in std::fs::read_dir(p.join("subagents")).into_iter().flatten().flatten() {
                if let Some(a) = s.file_name().to_string_lossy().strip_suffix(".jsonl") { out.push((format!("{name}/subagents/{a}"), size(&s.path()))); }
            } }
        }
        out
    }
    fn read_from(p: &Path, from: u64) -> Vec<u8> {
        use std::io::{Seek, SeekFrom};
        let Ok(mut f) = std::fs::File::open(p) else { return vec![] };
        if f.seek(SeekFrom::Start(from)).is_err() { return vec![] }
        let mut b = Vec::new(); let _ = f.take(CHUNK).read_to_end(&mut b); b
    }

    // ---------- the terminals Obsidian shows: per session, a tmux client on a pty attached to that Claude ----------

    /// What a session's pty printed: `start` is the offset of bytes[0] in everything it ever printed, so Obsidian asks
    /// for "from byte n" and never misses or repeats a byte while it keeps up. Offsets keep counting across restarts.
    struct Out { start: u64, bytes: Vec<u8> }
    impl Out { fn end(&self) -> u64 { self.start + self.bytes.len() as u64 } }
    /// `input` has a lock of its own: a write that blocks (a tmux client not reading) holds up that session only, not TERMS.
    struct Term { master: Box<dyn portable_pty::MasterPty + Send>, input: Arc<Mutex<Box<dyn Write + Send>>>, child: Box<dyn portable_pty::Child + Send + Sync> }
    /// By session name, as on a computer: session "x" is tmux session exo-x. "phone" (exo-phone) is the one session
    /// there was before sessions had names, and the one a request without a name means.
    static OUTS: Mutex<BTreeMap<String, Out>> = Mutex::new(BTreeMap::new());
    static TERMS: Mutex<BTreeMap<String, Term>> = Mutex::new(BTreeMap::new());

    /// A session name as the phone's panes make them (phone, phone-2, ...): letters, digits, - and _, 40 at most.
    fn name_ok(s: &str) -> bool { !s.is_empty() && s.len() <= 40 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') }
    /// A conversation id: Claude Code's uuid.
    fn id_ok(s: &str) -> bool { !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') }
    fn tmux_name(s: &str) -> String { format!("{PREFIX}{s}") }
    fn tmux(args: &[&str]) -> bool { std::process::Command::new("tmux").args(args).status().is_ok_and(|s| s.success()) }

    /// The same command as `exo-claude`: the session, made if it is not there, in the vault; `resume` picks up an earlier conversation.
    fn session_cmd(root: &Path, s: &str, resume: &str) -> Vec<String> {
        let mut v: Vec<String> = ["tmux", "new-session", "-A", "-s", &tmux_name(s), "-c"].iter().map(|x| x.to_string()).collect();
        // -n: the conversation's title is the session's name, which is how the pet tells which pane a log belongs to.
        // A Claude too old for it would exit at once on the unknown option, and no pane could start: then without it.
        v.extend([root.to_string_lossy().into_owned(), "claude".into()]);
        if names() { v.extend(["-n".into(), s.into()]); }
        if !resume.is_empty() { v.extend(["--resume".into(), resume.into()]); }
        v
    }
    /// Whether this VM's Claude takes `-n <name>` (Claude Code 2.1 does), asked once.
    fn names() -> bool {
        static NAMES: OnceLock<bool> = OnceLock::new();
        *NAMES.get_or_init(|| std::process::Command::new("claude").arg("--help").output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("--name")))
    }
    /// A session's terminal, started (or started again after Claude was quit) at this size.
    fn term(s: &str, cols: u16, rows: u16, resume: &str) -> Result<(), String> {
        let mut ts = TERMS.lock().unwrap();
        if let Some(x) = ts.get_mut(s) {
            if x.child.try_wait().ok().flatten().is_none() { return Ok(()) }
            ts.remove(s);
        }
        let pty = portable_pty::native_pty_system().openpty(portable_pty::PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).map_err(|e| e.to_string())?;
        let argv = session_cmd(ROOT.get().unwrap(), s, resume);
        let mut cmd = portable_pty::CommandBuilder::new(&argv[0]);
        cmd.args(&argv[1..]);
        cmd.cwd(ROOT.get().unwrap());
        cmd.env("TERM", "xterm-256color");
        let child = pty.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        let mut reader = pty.master.try_clone_reader().map_err(|e| e.to_string())?;
        let input = pty.master.take_writer().map_err(|e| e.to_string())?;
        let key = s.to_string();
        std::thread::spawn(move || {
            let mut b = [0u8; 8192];
            while let Ok(n) = reader.read(&mut b) {
                if n == 0 { break }
                let mut outs = OUTS.lock().unwrap();
                let o = outs.entry(key.clone()).or_insert(Out { start: 0, bytes: Vec::new() });
                o.bytes.extend_from_slice(&b[..n]);
                if o.bytes.len() > KEEP { let cut = o.bytes.len() - KEEP; o.bytes.drain(..cut); o.start += cut as u64; }
            }
        });
        ts.insert(s.into(), Term { master: pty.master, input: Arc::new(Mutex::new(input)), child });
        Ok(())
    }
    fn out_end(s: &str) -> u64 { OUTS.lock().unwrap().get(s).map_or(0, |o| o.end()) }
    /// A session's bytes after `from`, waiting up to `wait` for some: "<offset of the first byte>\n<bytes>". A pane with
    /// nothing new to show asks again only every few seconds, so a quiet Claude costs few requests.
    fn term_out(s: &str, from: u64, wait: Duration) -> Vec<u8> {
        let until = Instant::now() + wait;
        loop {
            {
                let outs = OUTS.lock().unwrap();
                let (start, bytes): (u64, &[u8]) = outs.get(s).map_or((0, &[]), |o| (o.start, &o.bytes));
                let at = from.clamp(start, start + bytes.len() as u64);
                if at < start + bytes.len() as u64 || Instant::now() >= until {
                    let mut r = format!("{at}\n").into_bytes();
                    r.extend_from_slice(&bytes[(at - start) as usize..]);
                    return r;
                }
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    // ---------- the Claude sessions window: what runs, earlier conversations ----------

    /// A JSON string in a log at `at` (just after its opening quote), decoded.
    fn json_string_at(t: &str, at: usize) -> Option<String> {
        let b = t.as_bytes(); let mut i = at;
        while i < b.len() { match b[i] { b'\\' => i += 2, b'"' => return serde_json::from_str(&t[at - 1..=i]).ok(), _ => i += 1 } }
        None
    }
    fn read_span(p: &Path, start: u64, len: usize) -> String {
        use std::io::{Seek, SeekFrom};
        let Ok(mut f) = std::fs::File::open(p) else { return String::new() };
        let mut b = vec![0u8; len];
        let n = f.seek(SeekFrom::Start(start)).and_then(|_| f.read(&mut b)).unwrap_or(0);
        String::from_utf8_lossy(&b[..n]).into_owned()
    }
    fn walk(p: &Path) -> u64 {
        match std::fs::symlink_metadata(p) {
            Ok(m) if m.is_dir() => std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| walk(&e.path())).sum(),
            Ok(m) => m.len(),
            Err(_) => 0,
        }
    }
    /// A conversation's title as the computer's list has it (node.ts listConversations): its last rename, else its first
    /// message. Hook and command blocks, and the Import tab's headless runs, are not conversations to list.
    /// The last "customTitle" in a log's first 64 KB and last 256 KB: a rename is appended near the end.
    fn renamed(p: &Path, size: u64) -> Option<String> {
        const RENAME: &str = "\"customTitle\":\"";
        let tail = if size > 65536 { read_span(p, size.saturating_sub(262144), 262144) } else { String::new() };
        let head = read_span(p, 0, 65536);
        tail.rfind(RENAME).and_then(|i| json_string_at(&tail, i + RENAME.len())).or_else(|| head.rfind(RENAME).and_then(|i| json_string_at(&head, i + RENAME.len())))
    }
    fn titled(p: &Path, size: u64) -> Option<(String, bool)> {
        const FIRST: &str = "\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"";
        let named = renamed(p, size);
        let title = named.clone().or_else(|| { let head = read_span(p, 0, 65536); head.find(FIRST).and_then(|i| json_string_at(&head, i + FIRST.len())) })?;
        let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        if title.is_empty() || title.starts_with('<') || title.starts_with("A file is being archived") { return None }
        Some((title.chars().take(70).collect(), named.is_some()))
    }
    /// A log's last rename, kept by its size: the pet asks every few seconds about every log.
    fn title_of(id: &str, size: u64) -> Option<String> {
        static TITLES: Mutex<BTreeMap<String, (u64, Option<String>)>> = Mutex::new(BTreeMap::new());
        if let Some((n, t)) = TITLES.lock().unwrap().get(id) { if *n == size { return t.clone() } }
        let t = renamed(&logs_dir().join(format!("{id}.jsonl")), size);
        TITLES.lock().unwrap().insert(id.into(), (size, t.clone()));
        t
    }
    fn sessions_json(n: usize) -> String {
        let out = std::process::Command::new("tmux").args(["list-sessions", "-F", "#{session_name}\t#{session_attached}\t#{session_created}"]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
        let sessions: Vec<_> = out.lines().filter_map(|l| {
            let mut f = l.split('\t'); let name = f.next()?.strip_prefix(PREFIX)?;
            if name.starts_with("view-") { return None }
            Some(serde_json::json!({ "name": name, "attached": f.next()? != "0", "created": f.next()?.parse::<u64>().unwrap_or(0) * 1000 }))
        }).collect();
        let dir = logs_dir();
        let mut convs: Vec<(u64, serde_json::Value)> = std::fs::read_dir(&dir).into_iter().flatten().flatten().filter_map(|e| {
            let p = e.path(); let id = e.file_name().to_string_lossy().strip_suffix(".jsonl")?.to_string();
            let m = e.metadata().ok()?; if m.len() < 200 { return None }
            let mtime = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64;
            let (title, named) = titled(&p, m.len())?;
            Some((mtime, serde_json::json!({ "id": id, "title": title, "mtime": mtime, "named": named, "size": m.len() + walk(&dir.join(&id)) })))
        }).collect();
        convs.sort_by(|a, b| b.0.cmp(&a.0)); convs.truncate(n);
        serde_json::json!({ "sessions": sessions, "conversations": convs.into_iter().map(|c| c.1).collect::<Vec<_>>() }).to_string()
    }

    fn routes(route: &str, param: &dyn Fn(&str) -> String, body: &[u8]) -> Option<(u16, Vec<u8>)> {
        let s = match param("session") { v if v.is_empty() => "phone".to_string(), v => v };
        if !name_ok(&s) { return Some((400, b"That is not a session name.".to_vec())) }
        Some(match route {
            "/pair" => {
                if !take_code(&param("code")) { return Some((403, b"That code is not the one shown in the Terminal, or it was used or is over ten minutes old. Run aiwalk-claude-vm pair there for a new one.".to_vec())) }
                eprintln!("Obsidian is paired.");
                (200, TOKEN.get().unwrap().clone().into_bytes())
            }
            "/cite" => {
                let text = String::from_utf8_lossy(body).into_owned();
                let target = format!("={}:", tmux_name(&s));   // ":": a session as a pane target, which send-keys needs
                if !tmux(&["has-session", "-t", &target]) { return Some((404, b"Claude is not running in the Terminal: open Claude in Obsidian, or run exo-claude there.".to_vec())) }
                // -l: the text as typed, so a note path is never read as a tmux key name; no Enter, the person sends it
                if tmux(&["send-keys", "-t", &target, "-l", "--", &text]) { (200, b"cited".to_vec()) } else { (400, b"tmux could not type into Claude.".to_vec()) }
            }
            // root: the vault as this VM sees it, which the paths in the logs start with
            // sessions: this bridge runs one Claude per session; title: a conversation's last rename (claude -n <session>),
            // how the pet tells whose a log is. Subagents' logs have none.
            "/logs" => (200, serde_json::json!({ "root": ROOT.get().unwrap().to_string_lossy(), "sessions": true,
                "logs": logs().iter().map(|(id, n)| match (!id.contains('/')).then(|| title_of(id, *n)).flatten() {
                    Some(t) => serde_json::json!({ "id": id, "size": n, "title": t }),
                    None => serde_json::json!({ "id": id, "size": n }),
                }).collect::<Vec<_>>() }).to_string().into_bytes()),
            "/log" => match log_path(&param("id")) { Some(p) => (200, read_from(&p, param("from").parse().unwrap_or(0))), None => (404, b"no such log".to_vec()) },
            "/sessions" => (200, sessions_json(param("n").parse().unwrap_or(50)).into_bytes()),
            "/session/end" => {
                let ok = tmux(&["kill-session", "-t", &format!("={}", tmux_name(&s))]);
                TERMS.lock().unwrap().remove(&s);
                if ok { (200, b"ended".to_vec()) } else { (404, b"no such session".to_vec()) }
            }
            // out of Claude's list without destroying it: the log moves to removed/ beside the others, as on a computer
            "/conversation/remove" => {
                let id = param("id"); if !id_ok(&id) { return Some((400, b"no such conversation".to_vec())) }
                let dir = logs_dir(); let _ = std::fs::create_dir_all(dir.join("removed"));
                match std::fs::rename(dir.join(format!("{id}.jsonl")), dir.join("removed").join(format!("{id}.jsonl"))) { Ok(()) => (200, b"removed".to_vec()), Err(e) => (404, e.to_string().into_bytes()) }
            }
            // opening a pane: start the session's terminal at the pane's size, or fit it; the answer is the offset to read
            // from. A resize makes tmux draw the whole screen again, so a pane that opens late still starts from a full screen.
            "/term/size" => {
                let (cols, rows) = (param("cols").parse().unwrap_or(80).clamp(20, 500), param("rows").parse().unwrap_or(24).clamp(5, 300));
                let resume = param("resume"); if !resume.is_empty() && !id_ok(&resume) { return Some((400, b"no such conversation".to_vec())) }
                let end = out_end(&s);
                if let Err(e) = term(&s, cols, rows, &resume) { return Some((400, format!("Could not start Claude: {e}").into_bytes())) }
                if let Some(t) = TERMS.lock().unwrap().get(&s) {
                    // a pane that reopens at the same size gets no redraw from a resize, so nudge it by a row first
                    let _ = t.master.resize(portable_pty::PtySize { rows: rows - 1, cols, pixel_width: 0, pixel_height: 0 });
                    let _ = t.master.resize(portable_pty::PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
                }
                (200, end.to_string().into_bytes())
            }
            "/term/out" => (200, term_out(&s, param("from").parse().unwrap_or(0), Duration::from_secs(8))),
            "/term/in" => {
                let mut ts = TERMS.lock().unwrap();
                // Claude was quit: the next key starts it again, as Enter does in a closed pane on a computer
                if ts.get_mut(&s).is_none_or(|x| x.child.try_wait().ok().flatten().is_some()) {
                    drop(ts);
                    if let Err(e) = term(&s, 80, 24, "") { return Some((400, format!("Could not start Claude: {e}").into_bytes())) }
                    return Some((200, b"started".to_vec()));
                }
                let input = ts.get(&s).unwrap().input.clone(); drop(ts);
                let r = input.lock().unwrap().write_all(body);
                match r { Ok(()) => (200, b"ok".to_vec()), Err(e) => (400, e.to_string().into_bytes()) }
            }
            _ => return None,
        })
    }

    /// `exo-claude` and the service, so the VM starts this program itself and a tab opens Claude with one word.
    fn install(root: &Path) -> Result<(), String> {
        let bin = home().join(".local/bin");
        std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        let here = bin.join("aiwalk-claude-vm");
        // a running service holds the old file open ("text file busy" when written over): a new file renamed onto it
        if me != here {
            let next = bin.join(".aiwalk-claude-vm.new");
            std::fs::copy(&me, &next).and_then(|_| std::fs::rename(&next, &here)).map_err(|e| format!("copying to {}: {e}", here.display()))?;
        }
        let q = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
        let script = format!("#!/bin/sh\n# Claude for this vault, in the tmux session Obsidian's Claude pane shows too. Ctrl-b d leaves it running.\nexec tmux new-session -A -s {PREFIX}phone -c {} claude \"$@\"\n", q(&root.to_string_lossy()));
        let exo = bin.join("exo-claude");
        std::fs::write(&exo, script).map_err(|e| e.to_string())?;
        { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(&exo, std::fs::Permissions::from_mode(0o755)); }
        let unit = home().join(".config/systemd/user/aiwalk-claude-vm.service");
        std::fs::create_dir_all(unit.parent().unwrap()).map_err(|e| e.to_string())?;
        // claude is in ~/.local/bin, which a service's PATH does not have. KillMode=process: a restart (an update) stops
        // this program only, not the tmux server it started, so Claude and its conversation keep running
        std::fs::write(&unit, format!("[Unit]\nDescription=Claude for Obsidian on this phone (aiwalk-claude-vm)\n\n[Service]\nExecStart={} {}\nEnvironment=PATH=%h/.local/bin:/usr/local/bin:/usr/bin:/bin\nRestart=on-failure\nKillMode=process\n\n[Install]\nWantedBy=default.target\n",
            here.display(), q(&root.to_string_lossy()))).map_err(|e| e.to_string())?;
        let run = |c: &[&str]| std::process::Command::new(c[0]).args(&c[1..]).status().is_ok_and(|s| s.success());
        // linger: the service starts with the VM, before anyone opens a tab
        let user = std::env::var("USER").unwrap_or_else(|_| "droid".into());
        if !run(&["sudo", "-n", "loginctl", "enable-linger", &user]) { eprintln!("Could not keep the service running without a tab open (sudo loginctl enable-linger {user}); it runs once a tab is open."); }
        // restart, not start: after an update the old program is still the one running
        if !(run(&["systemctl", "--user", "daemon-reload"]) && run(&["systemctl", "--user", "enable", "aiwalk-claude-vm.service"]) && run(&["systemctl", "--user", "restart", "aiwalk-claude-vm.service"])) {
            return Err("systemctl --user could not start the service".into());
        }
        Ok(())
    }

    pub fn main() {
        let mut args = std::env::args().skip(1);
        let first = args.next();
        if first.as_deref() == Some("pair") { println!("Pairing code for Obsidian: {} (good for ten minutes)", new_code()); return }
        let install_it = first.as_deref() == Some("install");
        let vault = if install_it { args.next() } else { first };
        let root = PathBuf::from(vault.unwrap_or(VAULT.into()));
        let Ok(root) = root.canonicalize() else { eprintln!("No vault at {}.", root.display()); std::process::exit(1) };
        if install_it {
            match install(&root) {
                Ok(()) => println!("Done. Claude for Obsidian starts with this VM; exo-claude opens the same Claude in a tab. Pairing code: {} (good for ten minutes): in Obsidian run \"Pair with Claude in the Linux Terminal\".", new_code()),
                Err(e) => { eprintln!("Install failed: {e}"); std::process::exit(1) }
            }
            return
        }
        let _ = ROOT.set(root);
        let token: &'static str = TOKEN.get_or_init(token).as_str();
        static VAULTS: crate::phonehttp::Vaults = crate::phonehttp::Vaults { file: |_| None, open: |_| Err(None), who: || None, more: routes };
        if !crate::phonehttp::serve(PORT, token, &VAULTS) { eprintln!("Port {PORT} is taken: is this already running?"); std::process::exit(1) }
        // as a service there is no one to read a code, and making one would replace the code `install` just showed
        use std::io::IsTerminal;
        if std::io::stderr().is_terminal() { eprintln!("Claude for Obsidian on port {PORT}. Already paired: nothing to do. To pair, in Obsidian run \"Pair with Claude in the Linux Terminal\" and type {} (good for ten minutes). Ctrl+C stops it.", new_code()); }
        else { eprintln!("Claude for Obsidian on port {PORT}. A pairing code: aiwalk-claude-vm pair"); }
        loop { std::thread::park() }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn a_code_works_once_and_only_as_given() {
            let dir = std::env::temp_dir().join(format!("claude-vm-test-{}", std::process::id()));
            std::env::set_var("HOME", &dir);
            let code = super::new_code();
            assert!(!super::take_code("000000x"));
            assert!(super::take_code(&code));
            assert!(!super::take_code(&code), "a code is good once");
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn the_terminal_hands_out_bytes_from_an_offset() {
            super::OUTS.lock().unwrap().insert("t".into(), super::Out { start: 10, bytes: b"hello".to_vec() });
            let w = std::time::Duration::from_millis(50);
            assert_eq!(super::term_out("t", 12, w), b"12\nllo");
            // an offset from before what is kept starts at the oldest byte kept; one past the end waits, then is empty
            assert_eq!(super::term_out("t", 0, w), b"10\nhello");
            assert_eq!(super::term_out("t", 99, w), b"15\n");
            // a session that printed nothing yet
            assert_eq!(super::term_out("none", 5, w), b"0\n");
        }

        #[test]
        fn names_and_titles_as_on_a_computer() {
            assert!(super::name_ok("phone") && super::name_ok("phone-2") && super::name_ok("a_b") && super::name_ok(&"x".repeat(40)));
            assert!(!super::name_ok("a b") && !super::name_ok("a.b") && !super::name_ok("") && !super::name_ok("$(x)") && !super::name_ok(&"x".repeat(41)) && !super::name_ok("篩檢"));
            // -n only where this computer's Claude has it (CI has no Claude)
            let cmd = super::session_cmd(std::path::Path::new("/v"), "phone-2", "");
            assert_eq!(cmd[4..8], ["exo-phone-2", "-c", "/v", "claude"]);
            assert_eq!(cmd[8..].to_vec(), if super::names() { vec!["-n", "phone-2"] } else { vec![] });
            assert!(super::id_ok("0a1b-2c") && !super::id_ok("../x"));
            let dir = std::env::temp_dir().join(format!("claude-vm-title-{}", std::process::id())); let _ = std::fs::create_dir_all(&dir);
            let p = dir.join("c.jsonl");
            let first = r#"{"type":"user","message":{"role":"user","content":"fix  the \"pet\"\nshop"}}"#;
            std::fs::write(&p, format!("{first}\n{}\n", "x".repeat(300))).unwrap();
            assert_eq!(super::titled(&p, std::fs::metadata(&p).unwrap().len()), Some(("fix the \"pet\" shop".into(), false)));
            std::fs::write(&p, format!("{first}\n{{\"type\":\"custom-title\",\"customTitle\":\"Pet shop\"}}\n")).unwrap();
            assert_eq!(super::titled(&p, std::fs::metadata(&p).unwrap().len()), Some(("Pet shop".into(), true)));
            // the last rename wins, escaped quotes undone; a log never renamed has no title for the pet
            std::fs::write(&p, format!("{{\"customTitle\":\"old\"}}\n{{\"customTitle\":\"say \\\"hi\\\"\"}}\n")).unwrap();
            assert_eq!(super::renamed(&p, std::fs::metadata(&p).unwrap().len()).as_deref(), Some("say \"hi\""));
            std::fs::write(&p, format!("{first}\n")).unwrap();
            assert_eq!(super::renamed(&p, std::fs::metadata(&p).unwrap().len()), None);
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    vm::main();
    #[cfg(not(target_os = "linux"))]
    eprintln!("aiwalk-claude-vm runs in Android's Linux Terminal.");
}
