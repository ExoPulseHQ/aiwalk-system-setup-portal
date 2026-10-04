//! `aiwalk-setup pty -- <command> [args...]`: a pseudo-terminal bridge, for the vault plugin's Claude pane.
//! Bytes on stdin go to the terminal, the terminal's bytes come out on stdout, and the exit status is the
//! command's. It replaces the plugin's pty_bridge.py, which needs Python modules Windows does not have; this one
//! uses the operating system's own pseudo-terminal on every OS (ConPTY on Windows).
//!
//! Size: the first size comes from the environment (PTY_COLS, PTY_ROWS), so the command starts at the pane's real
//! size. Later sizes arrive two ways, both accepted everywhere:
//!   - lines "resize COLS ROWS" on file descriptor 3, as pty_bridge.py took them (Unix only);
//!   - in the stdin stream itself, as ESC ] 777 ; resize ; COLS ; ROWS BEL, which is taken out before the rest goes
//!     to the terminal. Windows has no practical fd 3, so the plugin uses this one there.

use exo_core::ResizeScanner;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

fn size(cols: u16, rows: u16) -> PtySize { PtySize { rows, cols, pixel_width: 0, pixel_height: 0 } }

/// Runs the bridge; the process exit code to use.
pub fn main(args: &[String]) -> i32 {
    let cmd: Vec<&String> = args.iter().skip_while(|a| *a != "--").skip(1).collect();
    let Some(program) = cmd.first() else { eprintln!("usage: aiwalk-setup pty -- <command> [args...]"); return 2 };
    #[cfg(unix)]
    let ctl = take_fd3();
    let env = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<u16>().ok()).filter(|n| *n > 0);
    let pair = match native_pty_system().openpty(size(env("PTY_COLS").unwrap_or(80), env("PTY_ROWS").unwrap_or(24))) {
        Ok(p) => p,
        Err(e) => { eprintln!("this computer gave no pseudo-terminal: {e}"); return 1 }
    };
    let mut c = CommandBuilder::new(program.as_str());
    c.args(cmd[1..].iter().map(|a| a.as_str()));
    if let Ok(dir) = std::env::current_dir() { c.cwd(dir); }
    c.env("TERM", "xterm-256color");
    c.env("COLORTERM", "truecolor");
    let mut child = match pair.slave.spawn_command(c) {
        Ok(c) => c,
        Err(e) => { eprintln!("cannot start {program}: {e}"); return 127 }
    };
    drop(pair.slave);   // only the command holds the terminal's other end now, so its exit ends the reads below
    let (Ok(mut from_term), Ok(mut to_term)) = (pair.master.try_clone_reader(), pair.master.take_writer()) else {
        eprintln!("the pseudo-terminal could not be read"); return 1
    };
    let master: Arc<Mutex<Box<dyn MasterPty + Send>>> = Arc::new(Mutex::new(pair.master));
    let resize = { let m = master.clone(); move |cols: u16, rows: u16| { if cols > 0 && rows > 0 { let _ = m.lock().unwrap().resize(size(cols, rows)); } } };

    // terminal -> stdout; `done` says the terminal has nothing more
    let (done_tx, done) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let (mut buf, mut stdout) = (vec![0u8; 65536], std::io::stdout().lock());
        while let Ok(n) = from_term.read(&mut buf) { if n == 0 || stdout.write_all(&buf[..n]).and_then(|_| stdout.flush()).is_err() { break } }
        let _ = done_tx.send(());
    });
    // stdin -> terminal, resize requests taken out on the way. When the plugin closes stdin the command is hung up.
    let mut killer = child.clone_killer();
    {
        let resize = resize.clone();
        std::thread::spawn(move || {
            let (mut buf, mut stdin, mut scan) = (vec![0u8; 65536], std::io::stdin().lock(), ResizeScanner::default());
            loop {
                match stdin.read(&mut buf) {
                    Ok(n) if n > 0 => {
                        let (bytes, sizes) = scan.feed(&buf[..n]);
                        for (cols, rows) in sizes { resize(cols, rows) }
                        if !bytes.is_empty() && to_term.write_all(&bytes).and_then(|_| to_term.flush()).is_err() { break }
                    }
                    _ => { let _ = killer.kill(); break }
                }
            }
        });
    }
    #[cfg(unix)]
    if let Some(ctl) = ctl { watch_fd3(ctl, resize) }
    let code = child.wait().map(|s| s.exit_code() as i32).unwrap_or(1);
    // the command is gone: let its last output through. On Unix the read ends by itself; on Windows it ends only
    // when the terminal is closed, which other threads still hold, so the wait is bounded and the caller exits.
    drop(master);
    let _ = done.recv_timeout(std::time::Duration::from_millis(1500));
    code
}

/// fd 3, the caller's resize channel, if it is open. Call it before anything here opens a file: the pseudo-terminal
/// or a socket would otherwise land on 3 and be mistaken for it, and its bytes read away.
#[cfg(unix)]
pub fn take_fd3() -> Option<std::fs::File> {
    use std::os::fd::BorrowedFd;
    // SAFETY: fd 3 is only duplicated; nothing is read or closed through the borrowed handle
    unsafe { BorrowedFd::borrow_raw(3) }.try_clone_to_owned().ok().map(std::fs::File::from)
}

/// Calls `resize` for each "resize COLS ROWS" line on the caller's fd 3, on a thread of its own.
#[cfg(unix)]
pub fn watch_fd3(ctl: std::fs::File, resize: impl Fn(u16, u16) + Send + 'static) {
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(ctl).lines().map_while(Result::ok) {
            let p: Vec<&str> = line.split_whitespace().collect();
            if let ["resize", cols, rows] = p[..] { if let (Ok(c), Ok(r)) = (cols.parse(), rows.parse()) { resize(c, r) } }
        }
    });
}
