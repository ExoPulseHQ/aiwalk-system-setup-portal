//! `aiwalk-claude-vm [vault]`: Claude Code in Android's Linux Terminal, for Obsidian on the same phone. Claude runs in
//! the tmux session `exo-phone` (what `exo-claude` starts), beside Obsidian in split screen; this answers Obsidian on
//! port 47172, which the Terminal forwards to the phone. `POST /cite` types its body into Claude's prompt (not sent);
//! `GET /logs` lists the vault's conversation logs (and the vault's path here) and `GET /log?id=&from=` gives one from a byte offset, so the pet
//! follows what Claude edits. Typing into Claude is what a stolen token would be used for, so the token is not in the
//! vault, where any app with storage access reads it: it stays in this VM (~/.config/aiwalk/claude-vm-token), and
//! Obsidian gets it once, by the 6-digit code printed here (`GET /pair?code=`), and keeps it on the phone.

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
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    const PORT: u16 = 47172;
    const VAULT: &str = "/mnt/shared/Documents/aIwalk/exo-book";
    const SESSION: &str = "exo-phone";
    /// One read of a log at most: the pet asks again for the rest.
    const CHUNK: u64 = 4 << 20;

    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    /// The pairing code and when it was made; taken (None) once used, and good for ten minutes.
    static CODE: Mutex<Option<(String, std::time::Instant)>> = Mutex::new(None);
    static TOKEN: OnceLock<String> = OnceLock::new();

    fn home() -> PathBuf { PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/root".into())) }
    /// Claude Code's folder for this vault: its path with every non-alphanumeric character turned into "-".
    fn logs_dir() -> PathBuf {
        let v: String = ROOT.get().unwrap().to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        home().join(".claude").join("projects").join(v)
    }
    /// The token, made once and kept in this VM only (0600).
    fn token() -> String {
        let p = home().join(".config/aiwalk/claude-vm-token");
        if let Ok(t) = std::fs::read_to_string(&p) { if t.trim().len() == 32 { return t.trim().into() } }
        let t = crate::phonehttp::token();
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = std::fs::write(&p, &t);
        { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)); }
        t
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
        use std::io::{Read, Seek, SeekFrom};
        let Ok(mut f) = std::fs::File::open(p) else { return vec![] };
        if f.seek(SeekFrom::Start(from)).is_err() { return vec![] }
        let mut b = Vec::new(); let _ = f.take(CHUNK).read_to_end(&mut b); b
    }
    fn json_str(s: &str) -> String { format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")) }

    fn routes(route: &str, param: &dyn Fn(&str) -> String, body: &[u8]) -> Option<(u16, Vec<u8>)> {
        Some(match route {
            "/pair" => {
                let mut c = CODE.lock().unwrap();
                let fresh = c.as_ref().is_some_and(|(code, at)| at.elapsed().as_secs() < 600 && *code == param("code"));
                if !fresh { return Some((403, b"That code is not the one shown in the Terminal, or it was used or is over ten minutes old. Run aiwalk-claude-vm again for a new one.".to_vec())) }
                *c = None;
                eprintln!("Obsidian is paired.");
                (200, TOKEN.get().unwrap().clone().into_bytes())
            }
            "/cite" => {
                let text = String::from_utf8_lossy(body).into_owned();
                let has = std::process::Command::new("tmux").args(["has-session", "-t", SESSION]).status().is_ok_and(|s| s.success());
                if !has { return Some((404, b"Claude is not running in the Terminal: run exo-claude there first.".to_vec())) }
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
            _ => return None,
        })
    }

    pub fn main() {
        let root = PathBuf::from(std::env::args().nth(1).unwrap_or(VAULT.into()));
        let Ok(root) = root.canonicalize() else { eprintln!("No vault at {}.", root.display()); std::process::exit(1) };
        let _ = ROOT.set(root);
        let token: &'static str = TOKEN.get_or_init(token).as_str();
        static VAULTS: crate::phonehttp::Vaults = crate::phonehttp::Vaults { file: |_| None, open: |_| Err(None), who: || None, more: routes };
        if !crate::phonehttp::serve(PORT, token, &VAULTS) { eprintln!("Port {PORT} is taken: is this already running?"); std::process::exit(1) }
        let code = format!("{:06}", u32::from_le_bytes(crate::phonehttp::token().as_bytes()[..4].try_into().unwrap()) % 1_000_000);
        *CODE.lock().unwrap() = Some((code.clone(), std::time::Instant::now()));
        eprintln!("Claude for Obsidian on port {PORT}. Already paired: nothing to do. To pair, in Obsidian run \"Pair with Claude in the Linux Terminal\" and type {code} (good for ten minutes). Ctrl+C stops it.");
        loop { std::thread::park() }
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    vm::main();
    #[cfg(not(target_os = "linux"))]
    eprintln!("aiwalk-claude-vm runs in Android's Linux Terminal.");
}
