//! `aiwalk-claude-vm [vault]`: Claude Code in Android's Linux Terminal, for Obsidian on the same phone. Claude runs in
//! the tmux session `exo-phone` (what `exo-claude` starts); this answers Obsidian on port 47172, which the Terminal
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
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    const PORT: u16 = 47172;
    const VAULT: &str = "/mnt/shared/Documents/aIwalk/exo-book";
    const SESSION: &str = "exo-phone";
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
    fn json_str(s: &str) -> String { format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")) }

    // ---------- the terminal Obsidian shows: a tmux client on a pty, attached to Claude's session ----------

    /// What the pty printed: `start` is the offset of out[0] in everything it ever printed, so Obsidian asks for
    /// "from byte n" and never misses or repeats a byte while it keeps up. Offsets keep counting across restarts.
    struct Out { start: u64, bytes: Vec<u8> }
    impl Out { fn end(&self) -> u64 { self.start + self.bytes.len() as u64 } }
    struct Term { master: Box<dyn portable_pty::MasterPty + Send>, input: Box<dyn Write + Send>, child: Box<dyn portable_pty::Child + Send + Sync> }
    static OUT: Mutex<Out> = Mutex::new(Out { start: 0, bytes: Vec::new() });
    static TERM: Mutex<Option<Term>> = Mutex::new(None);

    /// The same command as `exo-claude`: Claude's session, made if it is not there, in the vault.
    fn session_cmd(root: &Path) -> Vec<String> {
        ["tmux", "new-session", "-A", "-s", SESSION, "-c"].iter().map(|s| s.to_string()).chain([root.to_string_lossy().into_owned(), "claude".into()]).collect()
    }
    /// The terminal, started (or started again after Claude was quit) at this size.
    fn term(cols: u16, rows: u16) -> Result<(), String> {
        let mut t = TERM.lock().unwrap();
        if let Some(x) = t.as_mut() {
            if x.child.try_wait().ok().flatten().is_none() { return Ok(()) }
            *t = None;
        }
        let pty = portable_pty::native_pty_system().openpty(portable_pty::PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).map_err(|e| e.to_string())?;
        let argv = session_cmd(ROOT.get().unwrap());
        let mut cmd = portable_pty::CommandBuilder::new(&argv[0]);
        cmd.args(&argv[1..]);
        cmd.cwd(ROOT.get().unwrap());
        cmd.env("TERM", "xterm-256color");
        let child = pty.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        let mut reader = pty.master.try_clone_reader().map_err(|e| e.to_string())?;
        let input = pty.master.take_writer().map_err(|e| e.to_string())?;
        std::thread::spawn(move || {
            let mut b = [0u8; 8192];
            while let Ok(n) = reader.read(&mut b) {
                if n == 0 { break }
                let mut o = OUT.lock().unwrap();
                o.bytes.extend_from_slice(&b[..n]);
                if o.bytes.len() > KEEP { let cut = o.bytes.len() - KEEP; o.bytes.drain(..cut); o.start += cut as u64; }
            }
        });
        *t = Some(Term { master: pty.master, input, child });
        Ok(())
    }
    /// The bytes after `from`, waiting up to a second for some: "<offset of the first byte>\n<bytes>".
    fn term_out(from: u64) -> Vec<u8> {
        let until = Instant::now() + Duration::from_secs(1);
        loop {
            {
                let o = OUT.lock().unwrap();
                let at = from.clamp(o.start, o.end());
                if at < o.end() || Instant::now() >= until {
                    let mut r = format!("{at}\n").into_bytes();
                    r.extend_from_slice(&o.bytes[(at - o.start) as usize..]);
                    return r;
                }
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    fn routes(route: &str, param: &dyn Fn(&str) -> String, body: &[u8]) -> Option<(u16, Vec<u8>)> {
        Some(match route {
            "/pair" => {
                if !take_code(&param("code")) { return Some((403, b"That code is not the one shown in the Terminal, or it was used or is over ten minutes old. Run aiwalk-claude-vm pair there for a new one.".to_vec())) }
                eprintln!("Obsidian is paired.");
                (200, TOKEN.get().unwrap().clone().into_bytes())
            }
            "/cite" => {
                let text = String::from_utf8_lossy(body).into_owned();
                let has = std::process::Command::new("tmux").args(["has-session", "-t", SESSION]).status().is_ok_and(|s| s.success());
                if !has { return Some((404, b"Claude is not running in the Terminal: open Claude in Obsidian, or run exo-claude there.".to_vec())) }
                // -l: the text as typed, so a note path is never read as a tmux key name; no Enter, the person sends it
                match std::process::Command::new("tmux").args(["send-keys", "-t", SESSION, "-l", "--", &text]).status() {
                    Ok(s) if s.success() => (200, b"cited".to_vec()),
                    _ => (400, b"tmux could not type into Claude.".to_vec()),
                }
            }
            // root: the vault as this VM sees it, which the paths in the logs start with
            "/logs" => (200, format!("{{\"root\":{},\"logs\":[{}]}}", json_str(&ROOT.get().unwrap().to_string_lossy()),
                logs().iter().map(|(id, n)| format!("{{\"id\":{},\"size\":{n}}}", json_str(id))).collect::<Vec<_>>().join(",")).into_bytes()),
            "/log" => match log_path(&param("id")) { Some(p) => (200, read_from(&p, param("from").parse().unwrap_or(0))), None => (404, b"no such log".to_vec()) },
            // opening the pane: start the terminal at the pane's size, or fit it; the answer is the offset to read from.
            // A resize makes tmux draw the whole screen again, so a pane that opens late still starts from a full screen.
            "/term/size" => {
                let (cols, rows) = (param("cols").parse().unwrap_or(80).clamp(20, 500), param("rows").parse().unwrap_or(24).clamp(5, 300));
                let end = OUT.lock().unwrap().end();
                if let Err(e) = term(cols, rows) { return Some((400, format!("Could not start Claude: {e}").into_bytes())) }
                if let Some(t) = TERM.lock().unwrap().as_ref() {
                    // a pane that reopens at the same size gets no redraw from a resize, so nudge it by a row first
                    let _ = t.master.resize(portable_pty::PtySize { rows: rows - 1, cols, pixel_width: 0, pixel_height: 0 });
                    let _ = t.master.resize(portable_pty::PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
                }
                (200, end.to_string().into_bytes())
            }
            "/term/out" => (200, term_out(param("from").parse().unwrap_or(0))),
            "/term/in" => {
                let mut t = TERM.lock().unwrap();
                // Claude was quit: the next key starts it again, as Enter does in a closed pane on a computer
                if t.as_mut().is_none_or(|x| x.child.try_wait().ok().flatten().is_some()) {
                    drop(t);
                    if let Err(e) = term(80, 24) { return Some((400, format!("Could not start Claude: {e}").into_bytes())) }
                    return Some((200, b"started".to_vec()));
                }
                match t.as_mut().unwrap().input.write_all(body) { Ok(()) => (200, b"ok".to_vec()), Err(e) => (400, e.to_string().into_bytes()) }
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
        if me != here { std::fs::copy(&me, &here).map_err(|e| format!("copying to {}: {e}", here.display()))?; }
        let q = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
        let script = format!("#!/bin/sh\n# Claude for this vault, in the tmux session Obsidian's Claude pane shows too. Ctrl-b d leaves it running.\nexec tmux new-session -A -s {SESSION} -c {} claude \"$@\"\n", q(&root.to_string_lossy()));
        let exo = bin.join("exo-claude");
        std::fs::write(&exo, script).map_err(|e| e.to_string())?;
        { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(&exo, std::fs::Permissions::from_mode(0o755)); }
        let unit = home().join(".config/systemd/user/aiwalk-claude-vm.service");
        std::fs::create_dir_all(unit.parent().unwrap()).map_err(|e| e.to_string())?;
        // claude is in ~/.local/bin, which a service's PATH does not have
        std::fs::write(&unit, format!("[Unit]\nDescription=Claude for Obsidian on this phone (aiwalk-claude-vm)\n\n[Service]\nExecStart={} {}\nEnvironment=PATH=%h/.local/bin:/usr/local/bin:/usr/bin:/bin\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n",
            here.display(), q(&root.to_string_lossy()))).map_err(|e| e.to_string())?;
        let run = |c: &[&str]| std::process::Command::new(c[0]).args(&c[1..]).status().is_ok_and(|s| s.success());
        // linger: the service starts with the VM, before anyone opens a tab
        let user = std::env::var("USER").unwrap_or_else(|_| "droid".into());
        if !run(&["sudo", "-n", "loginctl", "enable-linger", &user]) { eprintln!("Could not keep the service running without a tab open (sudo loginctl enable-linger {user}); it runs once a tab is open."); }
        if !(run(&["systemctl", "--user", "daemon-reload"]) && run(&["systemctl", "--user", "enable", "--now", "aiwalk-claude-vm.service"])) {
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
            { let mut o = super::OUT.lock().unwrap(); o.start = 10; o.bytes = b"hello".to_vec(); }
            assert_eq!(super::term_out(12), b"12\nllo");
            // an offset from before what is kept starts at the oldest byte kept; one past the end waits, then is empty
            assert_eq!(super::term_out(0), b"10\nhello");
            assert_eq!(super::term_out(99), b"15\n");
        }
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    vm::main();
    #[cfg(not(target_os = "linux"))]
    eprintln!("aiwalk-claude-vm runs in Android's Linux Terminal.");
}
