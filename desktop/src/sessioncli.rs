//! `aiwalk-setup session`: terminal sessions that outlive the pane showing them, what the vault plugin used tmux for.
//!
//!   session open NAME [--cwd DIR] -- <command> [args...]   attach to NAME, starting <command> in it if NAME is new
//!   session list                                            JSON array of the live sessions
//!   session send NAME                                       stdin goes into NAME's input as a paste, not submitted
//!   session end NAME                                        stop NAME's command; exit 0 once it is gone
//!
//! While `open` is attached, stdin and stdout are the terminal, as with `aiwalk-setup pty` (ptycli.rs): the first
//! size comes from PTY_COLS / PTY_ROWS, later sizes in-band as ESC ] 777 ; resize ; COLS ; ROWS BEL or on fd 3 as
//! "resize COLS ROWS" lines. The caller leaving (stdin or stdout closed, killed) only detaches. When the command
//! exits, every attached caller exits with its exit code and the session is gone.
//!
//! A session is a keeper: this binary re-run as `session keep NAME CWD COLS ROWS -- <command...>`, detached from
//! whoever started it (its own session on Unix, a detached process on Windows), holding the pseudo-terminal and
//! listening on a Unix socket or named pipe that only this user can open. It keeps the last 256 KB of output to
//! replay to a pane that attaches, tracks the window title, and mirrors the terminal to every attached client; the
//! terminal takes the size of whichever client resized last. Clients speak to it in frames (exo_core::session).
//!
//! Replay limits: the 256 KB starts at the first whole character or sequence after the cut, preceded by the private
//! modes still on (alternate screen, bracketed paste, mouse ...) and without the questions programs ask the terminal.
//! Colours or cursor positions set before the cut are not restored, so a shell's old lines may show uncoloured; a
//! full-screen program is asked to repaint (the size is changed by one row and back) and so shows correctly.

use exo_core::session::{self as s, kind, Screen};
use exo_core::ResizeScanner;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{sleep, spawn};
use std::time::{Duration, Instant};

const KEEP: usize = 256 * 1024;
const USAGE: &str = "usage: aiwalk-setup session open NAME [--cwd DIR] -- <command> [args...] | list | send NAME | end NAME";

fn size(cols: u16, rows: u16) -> PtySize { PtySize { rows, cols, pixel_width: 0, pixel_height: 0 } }

pub fn main(args: &[String]) -> i32 {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let name = || match a.get(1) {
        Some(n) if s::valid_name(n) => Some(*n),
        Some(n) => { eprintln!("bad session name {n:?}: use 1 to 64 of A-Z a-z 0-9 . _ -"); None }
        None => { eprintln!("{USAGE}"); None }
    };
    match a.first().copied() {
        Some("open") => name().map_or(2, |n| open(n, &a[2..])),
        Some("list") => list(),
        Some("send") => name().map_or(2, send),
        Some("end") => name().map_or(2, end),
        Some("keep") => name().map_or(2, |n| keep(n, &a[2..])),
        _ => { eprintln!("{USAGE}"); 2 }
    }
}

// ---------------------------------------------------------------- clients

/// A connection to session `name`'s keeper, past its greeting.
fn dial(name: &str) -> io::Result<os::Conn> {
    let mut c = os::connect(name)?;
    match s::read_frame(&mut c)? {
        Some((kind::HELLO, key)) if os::hello_ok(name, &key) => Ok(c),
        Some(_) => Err(io::Error::new(io::ErrorKind::PermissionDenied, "the session is held by something other than this user's keeper")),
        None => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the session's keeper hung up")),
    }
}

fn put(w: &Mutex<os::Conn>, k: u8, body: &[u8]) -> bool { w.lock().unwrap().write_all(&s::frame(k, body)).is_ok() }

fn open(name: &str, rest: &[&str]) -> i32 {
    #[cfg(unix)]
    let ctl = crate::ptycli::take_fd3();   // first, before a socket can take fd 3
    let (mut cwd, mut cmd, mut it) = (None, vec![], rest.iter());
    while let Some(a) = it.next() {
        match *a {
            "--cwd" => cwd = it.next().copied(),
            "--" => { cmd = it.by_ref().copied().collect(); break }
            other => { eprintln!("unknown option {other}\n{USAGE}"); return 2 }
        }
    }
    let env = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<u16>().ok()).filter(|n| *n > 0);
    let (cols, rows) = (env("PTY_COLS").unwrap_or(80), env("PTY_ROWS").unwrap_or(24));

    // attach to the keeper, starting one when there is none. Two callers may race to start the same name: the one
    // whose keeper loses exits 3 and the caller attaches to the winner.
    let (deadline, mut keeper, mut started) = (Instant::now() + Duration::from_secs(10), None::<std::process::Child>, false);
    let mut conn = loop {
        match dial(name) {
            Ok(c) => break c,
            Err(e) if !started && os::gone(&e) => {
                if cmd.is_empty() { eprintln!("no session {name}, and no command to start one"); return 1 }
                started = true;
                match spawn_keeper(name, cwd, (cols, rows), &cmd) {
                    Ok(k) => keeper = Some(k),
                    Err(e) => { eprintln!("cannot start session {name}: {e}"); return 1 }
                }
            }
            Err(e) if !started => { eprintln!("session {name}: {e}"); return 1 }
            Err(e) => {
                if let Some(code) = keeper.as_mut().and_then(|k| k.try_wait().ok().flatten()).map(|s| s.code().unwrap_or(1)) {
                    keeper = None;
                    match code {
                        3 => {}
                        127 => { eprintln!("cannot start {}", cmd[0]); return 127 }
                        _ => { eprintln!("session {name}: its keeper failed (exit {code})"); return 1 }
                    }
                }
                if Instant::now() > deadline { eprintln!("session {name} did not come up: {e}"); return 1 }
                sleep(Duration::from_millis(20));
            }
        }
    };
    let Ok(w) = os::clone(&conn).map(|c| Arc::new(Mutex::new(c))) else { return 1 };
    if !put(&w, kind::ATTACH, &s::size_body(cols, rows)) { eprintln!("session {name}: its keeper hung up"); return 1 }

    // stdin -> keeper, sizes taken out on the way. When the caller lets go of stdin it is only detaching.
    {
        let w = w.clone();
        spawn(move || {
            let (mut buf, mut stdin, mut scan) = (vec![0u8; 65536], io::stdin().lock(), ResizeScanner::default());
            loop {
                match stdin.read(&mut buf) {
                    Ok(n) if n > 0 => {
                        let (bytes, sizes) = scan.feed(&buf[..n]);
                        for (c, r) in sizes { put(&w, kind::RESIZE, &s::size_body(c, r)); }
                        if !bytes.is_empty() && !put(&w, kind::INPUT, &bytes) { break }
                    }
                    _ => std::process::exit(0),
                }
            }
        });
    }
    #[cfg(unix)]
    if let Some(ctl) = ctl { crate::ptycli::watch_fd3(ctl, move |c, r| { put(&w, kind::RESIZE, &s::size_body(c, r)); }) }

    // keeper -> stdout, until the command exits (its code) or the caller stops reading (detach)
    let mut stdout = io::stdout().lock();
    loop {
        match s::read_frame(&mut conn) {
            Ok(Some((kind::OUTPUT, b))) => if stdout.write_all(&b).and_then(|_| stdout.flush()).is_err() { return 0 },
            Ok(Some((kind::EXIT, b))) => return b.get(..4).map_or(1, |b| i32::from_le_bytes(b.try_into().unwrap())),
            Ok(Some(_)) => {}
            _ => { eprintln!("session {name}: its keeper is gone"); return 1 }
        }
    }
}

/// Starts the keeper for a new session, detached from this process, its caller and their terminal.
fn spawn_keeper(name: &str, cwd: Option<&str>, (cols, rows): (u16, u16), cmd: &[&str]) -> io::Result<std::process::Child> {
    let cwd = std::path::absolute(cwd.map_or_else(std::env::current_dir, |d| Ok(d.into()))?)?;
    if !cwd.is_dir() { return Err(io::Error::new(io::ErrorKind::NotFound, format!("no folder {}", cwd.display()))) }
    // from an AppImage the binary lives in a mount that goes away with this process: start the AppImage again
    let exe = crate::own_appimage().map_or_else(std::env::current_exe, Ok)?;
    os::before_spawn();
    let mut c = std::process::Command::new(exe);
    c.args(["session", "keep", name]).arg(&cwd).args([cols.to_string(), rows.to_string()]).arg("--").args(cmd)
        .current_dir(&cwd).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    os::detach(&mut c);
    c.spawn()
}

fn list() -> i32 {
    let mut all = vec![];
    for name in os::names() {
        match dial(&name) {
            Ok(mut c) => {
                if c.write_all(&s::frame(kind::LIST, &[])).is_ok() {
                    if let Ok(Some((kind::INFO, b))) = s::read_frame(&mut c) { all.extend(serde_json::from_slice::<serde_json::Value>(&b)) }
                }
            }
            Err(e) if os::gone(&e) => os::forget(&name),   // its keeper died without cleaning up
            Err(_) => {}
        }
    }
    println!("{}", serde_json::Value::Array(all));
    0
}

fn send(name: &str) -> i32 {
    let mut c = match dial(name) {
        Ok(c) => c,
        Err(e) if os::gone(&e) => { eprintln!("no session {name}"); return 1 }
        Err(e) => { eprintln!("session {name}: {e}"); return 1 }
    };
    let mut text = vec![];
    if let Err(e) = io::stdin().read_to_end(&mut text) { eprintln!("cannot read stdin: {e}"); return 1 }
    if c.write_all(&s::frame(kind::PASTE, &text)).is_err() { eprintln!("session {name}: its keeper hung up"); return 1 }
    while let Ok(Some(_)) = s::read_frame(&mut c) {}   // the keeper hangs up once the text is in
    0
}

fn end(name: &str) -> i32 {
    match dial(name) {
        Ok(mut c) => {
            let _ = c.write_all(&s::frame(kind::END, &[]));
            while let Ok(Some(_)) = s::read_frame(&mut c) {}   // the keeper hangs up once the command is gone
        }
        Err(e) if os::gone(&e) => os::forget(name),
        Err(e) => { eprintln!("session {name}: {e}"); return 1 }
    }
    0
}

// ---------------------------------------------------------------- the keeper

struct Keeper {
    info: serde_json::Value,
    key: Vec<u8>,
    pid: Option<u32>,
    /// None once the terminal is closed
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    input: Mutex<Box<dyn Write + Send>>,
    shared: Mutex<Shared>,
    /// the command has exited
    exited: AtomicBool,
    /// ... and the session is no longer listed
    gone: AtomicBool,
    /// connections being served
    active: AtomicUsize,
}

struct Shared {
    screen: Screen,
    /// attached clients: id, the queue their writer thread sends from, the connection (to cut it off)
    clients: Vec<(u64, SyncSender<Vec<u8>>, os::Conn)>,
    attached_once: bool,
    /// the command's exit code, once every client has been told it
    exit: Option<i32>,
}

fn keep(name: &str, a: &[&str]) -> i32 {
    let num = |i: usize| a.get(i).and_then(|v| v.parse::<u16>().ok()).filter(|n| *n > 0);
    let (Some(cwd), Some(cols), Some(rows), Some("--"), Some(program)) = (a.first(), num(1), num(2), a.get(3).copied(), a.get(4)) else { return 2 };
    let cmd = &a[4..];
    let listener = match os::Listener::bind(name) {
        Ok(l) => Arc::new(l),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return 3,
        Err(_) => return 1,
    };
    let fail = |code| { listener.remove(); code };
    let Ok(pair) = native_pty_system().openpty(size(cols, rows)) else { return fail(1) };
    let mut c = CommandBuilder::new(program);
    c.args(&cmd[1..]);
    c.cwd(cwd);
    c.env("TERM", "xterm-256color");
    c.env("COLORTERM", "truecolor");
    let Ok(mut child) = pair.slave.spawn_command(c) else { return fail(127) };
    drop(pair.slave);
    let (Ok(mut from_term), Ok(to_term)) = (pair.master.try_clone_reader(), pair.master.take_writer()) else { let _ = child.kill(); return fail(1) };
    let created = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let k = Arc::new(Keeper {
        info: serde_json::json!({ "name": name, "cwd": cwd, "command": cmd, "created": created, "pid": child.process_id() }),
        key: listener.key(),
        pid: child.process_id(),
        master: Mutex::new(Some(pair.master)),
        input: Mutex::new(to_term),
        shared: Mutex::new(Shared { screen: Screen::new(KEEP), clients: vec![], attached_once: false, exit: None }),
        exited: AtomicBool::new(false),
        gone: AtomicBool::new(false),
        active: AtomicUsize::new(0),
    });

    // terminal -> the kept screen and every attached client
    let (done_tx, done) = std::sync::mpsc::channel::<()>();
    {
        let k = k.clone();
        spawn(move || {
            let mut buf = vec![0u8; 65536];
            while let Ok(n @ 1..) = from_term.read(&mut buf) {
                let f = s::frame(kind::OUTPUT, &buf[..n]);
                let mut sh = k.shared.lock().unwrap();
                sh.screen.push(&buf[..n]);
                // a client this far behind is cut off rather than allowed to stall the command; it can attach again
                sh.clients.retain(|(_, tx, conn)| tx.try_send(f.clone()).is_ok() || { os::cut(conn); false });
            }
            let _ = done_tx.send(());
        });
    }
    {
        let (k, l) = (k.clone(), listener.clone());
        spawn(move || while let Ok(c) = l.accept() {
            let k = k.clone();
            k.active.fetch_add(1, Ordering::SeqCst);
            spawn(move || { serve(&k, c); k.active.fetch_sub(1, Ordering::SeqCst) });
        });
    }

    let code = child.wait().map(|s| s.exit_code() as i32).unwrap_or(1);
    k.exited.store(true, Ordering::SeqCst);
    // let the last output through (on Windows the read ends only once the terminal is closed), then say goodbye
    drop(k.master.lock().unwrap().take());
    let _ = done.recv_timeout(Duration::from_millis(1500));
    {
        let mut sh = k.shared.lock().unwrap();
        sh.exit = Some(code);
        for (_, tx, _) in sh.clients.drain(..) { let _ = tx.try_send(s::frame(kind::EXIT, &code.to_le_bytes())); }
    }
    // a command that exits at once may beat its caller to attaching: the caller still gets its output and code
    let soon = |secs| Instant::now() + Duration::from_secs(secs);
    let until = soon(5);
    while !k.shared.lock().unwrap().attached_once && Instant::now() < until { sleep(Duration::from_millis(20)) }
    listener.remove();
    k.gone.store(true, Ordering::SeqCst);
    let until = soon(2);
    while k.active.load(Ordering::SeqCst) > 0 && Instant::now() < until { sleep(Duration::from_millis(20)) }
    code
}

/// One connection: greet, then do what its first frame asks.
fn serve(k: &Keeper, mut c: os::Conn) {
    if c.write_all(&s::frame(kind::HELLO, &k.key)).is_err() { return }
    match s::read_frame(&mut c) {
        Ok(Some((kind::ATTACH, b))) => attach(k, c, s::parse_size(&b)),
        Ok(Some((kind::PASTE, b))) => { let mut i = k.input.lock().unwrap(); let _ = i.write_all(&s::bracketed_paste(&b)).and_then(|_| i.flush()); }
        Ok(Some((kind::END, _))) => stop(k),
        Ok(Some((kind::LIST, _))) => {
            let mut info = k.info.clone();
            { let sh = k.shared.lock().unwrap(); info["title"] = sh.screen.title.clone().into(); info["attached"] = sh.clients.len().into(); }
            let _ = c.write_all(&s::frame(kind::INFO, info.to_string().as_bytes()));
        }
        _ => {}
    }
}

fn attach(k: &Keeper, mut c: os::Conn, size: Option<(u16, u16)>) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let (Ok(mut out), Ok(cutter)) = (os::clone(&c), os::clone(&c)) else { return };
    let (tx, rx) = sync_channel::<Vec<u8>>(256);
    // replay and joining the mirror happen under the lock that output is pushed under: nothing is lost or doubled
    // once the queue is dropped (detach, exit, too far behind) the writer ends the connection, and so this thread
    let had_output = {
        let mut sh = k.shared.lock().unwrap();
        sh.attached_once = true;
        let _ = tx.try_send(s::frame(kind::OUTPUT, &sh.screen.replay()));
        match sh.exit {
            Some(code) => { let _ = tx.try_send(s::frame(kind::EXIT, &code.to_le_bytes())); }
            None => sh.clients.push((id, tx, cutter)),
        }
        spawn(move || { for f in rx { if out.write_all(&f).is_err() { break } } os::cut(&out) });
        !sh.screen.is_empty()
    };
    if let Some((cols, rows)) = size { k.resize(cols, rows, had_output) }
    while let Ok(Some((t, b))) = s::read_frame(&mut c) {
        match t {
            kind::INPUT => { let mut i = k.input.lock().unwrap(); if i.write_all(&b).and_then(|_| i.flush()).is_err() { break } }
            kind::RESIZE => if let Some((cols, rows)) = s::parse_size(&b) { k.resize(cols, rows, false) },
            _ => break,
        }
    }
    k.shared.lock().unwrap().clients.retain(|(i, ..)| *i != id);
}

impl Keeper {
    /// Sizes the terminal. With `repaint`, also when the size is unchanged: one row less and back, so a full-screen
    /// program redraws for the pane that just attached.
    fn resize(&self, cols: u16, rows: u16, repaint: bool) {
        let m = self.master.lock().unwrap();
        let Some(m) = m.as_ref() else { return };
        if repaint && m.get_size().is_ok_and(|s| s.cols == cols && s.rows == rows) {
            let _ = m.resize(size(cols, if rows > 1 { rows - 1 } else { 2 }));
            sleep(Duration::from_millis(100));   // two changes in a row may reach the program as one, and no change
        }
        let _ = m.resize(size(cols, rows));
    }
}

/// `session end`: hang the command up politely, kill it (and what it started) if it is still there 3 s later, and
/// return once it is gone and the session no longer listed.
fn stop(k: &Keeper) {
    let wait = |flag: &AtomicBool, secs| { let until = Instant::now() + Duration::from_secs(secs); while !flag.load(Ordering::SeqCst) && Instant::now() < until { sleep(Duration::from_millis(50)) } };
    if let Some(pid) = k.pid {
        #[cfg(unix)]
        os::signal(pid, libc::SIGHUP);
        #[cfg(windows)]
        drop(k.master.lock().unwrap().take());   // closing the pseudo console sends its programs CTRL_CLOSE_EVENT
        wait(&k.exited, 3);
        if !k.exited.load(Ordering::SeqCst) {
            #[cfg(unix)]
            os::signal(pid, libc::SIGKILL);
            #[cfg(windows)]
            let _ = crate::cmd("taskkill").args(["/T", "/F", "/PID", &pid.to_string()]).output();
            wait(&k.exited, 3);
        }
    }
    wait(&k.gone, 8);
}

// ---------------------------------------------------------------- where sessions live

/// Unix: a socket per session in a folder only this user can enter.
#[cfg(unix)]
mod os {
    use std::io;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;

    pub type Conn = UnixStream;
    pub fn clone(c: &Conn) -> io::Result<Conn> { c.try_clone() }
    pub fn cut(c: &Conn) { let _ = c.shutdown(std::net::Shutdown::Both); }

    /// $XDG_RUNTIME_DIR/aiwalk-setup/sessions, else under ~/.local/state (Linux) or ~/Library/Application Support
    /// (macOS); mode 0700 and refused unless it is a real folder owned by this user.
    fn dir() -> io::Result<PathBuf> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let d = match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
            Some(r) => PathBuf::from(r).join("aiwalk-setup/sessions"),
            None if cfg!(target_os = "macos") => crate::home().join("Library/Application Support/aiwalk-setup/sessions"),
            None => crate::home().join(".local/state/aiwalk-setup/sessions"),
        };
        std::fs::create_dir_all(&d)?;
        let m = std::fs::symlink_metadata(&d)?;
        // SAFETY: getuid has no failure case
        if !m.is_dir() || m.uid() != unsafe { libc::getuid() } {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{} is not this user's own folder", d.display())));
        }
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700))?;
        Ok(d)
    }

    /// The socket of session `name`. A socket path holds at most 104 bytes on macOS (108 on Linux), which a long name
    /// under ~/Library/Application Support can pass: then the file is named by a hash of the name instead. Files are
    /// only ever found by name or listed, and list takes the name from the keeper, so either form works.
    fn path(name: &str) -> io::Result<PathBuf> {
        let d = dir()?;
        let p = d.join(format!("{name}.sock"));
        if p.as_os_str().len() <= 100 { return Ok(p) }
        let h = name.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));   // FNV-1a
        Ok(d.join(format!("{h:016x}.sock")))
    }

    pub fn connect(name: &str) -> io::Result<Conn> { UnixStream::connect(path(name)?) }

    /// No socket, or nobody listening on it: the session does not exist (any more).
    pub fn gone(e: &io::Error) -> bool { matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused) }
    pub fn forget(name: &str) { if let Ok(p) = path(name) { let _ = std::fs::remove_file(p); } }
    pub fn hello_ok(_: &str, _: &[u8]) -> bool { true }   // the folder's permissions already say who is listening

    /// The sessions on disk, live or not, by file name (a name, or the hash a long name was given).
    pub fn names() -> Vec<String> {
        let mut v: Vec<String> = dir().and_then(std::fs::read_dir).into_iter().flatten().flatten()
            .filter_map(|e| e.file_name().to_str()?.strip_suffix(".sock").map(String::from)).filter(|n| exo_core::session::valid_name(n)).collect();
        v.sort();
        v
    }

    pub struct Listener(UnixListener, PathBuf);

    impl Listener {
        /// Claims session `name`; AlreadyExists when a live keeper has it. A socket left by a dead keeper is replaced.
        // ponytail: between another keeper's bind and listen (microseconds) its socket looks dead and is replaced;
        // a lock file held with flock would close that window if two opens of one new name ever collide in practice.
        pub fn bind(name: &str) -> io::Result<Self> {
            let p = path(name)?;
            match UnixListener::bind(&p) {
                Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
                    if UnixStream::connect(&p).is_ok() { return Err(io::ErrorKind::AlreadyExists.into()) }
                    std::fs::remove_file(&p)?;
                    Ok(Listener(UnixListener::bind(&p)?, p))
                }
                l => Ok(Listener(l?, p)),
            }
        }
        pub fn accept(&self) -> io::Result<Conn> { self.0.accept().map(|(c, _)| c) }
        pub fn key(&self) -> Vec<u8> { vec![] }
        pub fn remove(&self) { let _ = std::fs::remove_file(&self.1); }
    }

    /// fd 3 (the caller's resize channel) is not to stay open in the keeper
    pub fn before_spawn() { unsafe { libc::fcntl(3, libc::F_SETFD, libc::FD_CLOEXEC) }; }

    /// A session of its own: no controlling terminal, and no hangup when the caller's terminal or process group goes.
    pub fn detach(c: &mut std::process::Command) {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe and touches nothing of the parent's
        unsafe { c.pre_exec(|| { libc::setsid(); Ok(()) }) };
    }

    /// `sig` to the command's process group (it leads one: portable-pty starts it with setsid) and to itself.
    pub fn signal(pid: u32, sig: i32) { unsafe { libc::kill(-(pid as i32), sig); libc::kill(pid as i32, sig); } }
}

/// Windows: a named pipe per session, \\.\pipe\aiwalk-setup-session-<user>-<NAME>, whose DACL admits only this user's
/// SID (the default DACL would let Everyone read), refusing remote clients. Squatting is the remaining risk: another
/// user could create the name first. So the keeper writes a random key to <LOCALAPPDATA>\aiwalk-setup\sessions\
/// <NAME>.pipe, a folder only this user can read, and greets every client with it; a client that is not greeted
/// with the key hangs up before sending anything. Clients open the pipe at identification level only, so a squatter
/// cannot impersonate them either.
#[cfg(windows)]
mod os {
    use std::io;
    use std::path::PathBuf;
    use std::ptr::{null, null_mut};
    use std::sync::{Arc, Mutex};
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::Storage::FileSystem::*;
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
    use windows_sys::Win32::System::Pipes::*;
    use windows_sys::Win32::System::Threading::{CreateEventW, GetCurrentProcess, OpenProcessToken};
    use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

    pub struct Pipe(HANDLE);
    // SAFETY: a pipe handle may be used from any thread; overlapped I/O (below) makes concurrent read and write safe
    unsafe impl Send for Pipe {}
    unsafe impl Sync for Pipe {}
    impl Drop for Pipe { fn drop(&mut self) { unsafe { CloseHandle(self.0) }; } }

    #[derive(Clone)]
    pub struct Conn(Arc<Pipe>);
    pub fn clone(c: &Conn) -> io::Result<Conn> { Ok(c.clone()) }
    pub fn cut(c: &Conn) { unsafe { CancelIoEx(c.0 .0, null()); DisconnectNamedPipe(c.0 .0) }; }

    /// One overlapped operation on `h`, waited for. The pipes are overlapped because Windows serializes all I/O on a
    /// synchronous handle: a read waiting for the keeper would block every write from the other thread.
    fn wait(h: HANDLE, op: impl FnOnce(*mut OVERLAPPED) -> BOOL) -> io::Result<u32> {
        unsafe {
            let ev = CreateEventW(null(), 1, 0, null());
            if ev.is_null() { return Err(io::Error::last_os_error()) }
            let mut ov: OVERLAPPED = std::mem::zeroed();
            ov.hEvent = ev;
            let mut n = 0u32;
            let r = if op(&mut ov) == 0 && GetLastError() != ERROR_IO_PENDING { Err(io::Error::last_os_error()) }
                else if GetOverlappedResult(h, &ov, &mut n, 1) == 0 { Err(io::Error::last_os_error()) }
                else { Ok(n) };
            CloseHandle(ev);
            r
        }
    }

    impl io::Read for Conn {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            let h = self.0 .0;
            let len = b.len().min(1 << 20) as u32;
            match wait(h, |ov| unsafe { ReadFile(h, b.as_mut_ptr(), len, null_mut(), ov) }) {
                Ok(n) => Ok(n as usize),
                Err(e) if [ERROR_BROKEN_PIPE, ERROR_PIPE_NOT_CONNECTED].iter().any(|c| e.raw_os_error() == Some(*c as i32)) => Ok(0),
                Err(e) => Err(e),
            }
        }
    }
    impl io::Write for Conn {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            let h = self.0 .0;
            let len = b.len().min(1 << 20) as u32;
            wait(h, |ov| unsafe { WriteFile(h, b.as_ptr(), len, null_mut(), ov) }).map(|n| n as usize)
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain([0]).collect() }

    fn pipe_name(name: &str) -> Vec<u16> {
        let user: String = std::env::var("USERNAME").unwrap_or_default().chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).collect();
        wide(&format!(r"\\.\pipe\aiwalk-setup-session-{user}-{name}"))
    }

    fn dir() -> io::Result<PathBuf> {
        let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| crate::home().join(r"AppData\Local"));
        let d = base.join(r"aiwalk-setup\sessions");
        std::fs::create_dir_all(&d)?;
        Ok(d)
    }
    fn marker(name: &str) -> io::Result<PathBuf> { Ok(dir()?.join(format!("{name}.pipe"))) }

    pub fn connect(name: &str) -> io::Result<Conn> {
        let pipe = pipe_name(name);
        for _ in 0..100 {
            let h = unsafe {
                CreateFileW(pipe.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, null(), OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION, null_mut())
            };
            if h != INVALID_HANDLE_VALUE { return Ok(Conn(Arc::new(Pipe(h)))) }
            let e = io::Error::last_os_error();
            if e.raw_os_error() != Some(ERROR_PIPE_BUSY as i32) { return Err(e) }
            std::thread::sleep(std::time::Duration::from_millis(20));   // every instance is taken: the keeper is making one
        }
        Err(io::Error::new(io::ErrorKind::TimedOut, "the session's pipe stayed busy"))
    }

    pub fn gone(e: &io::Error) -> bool { e.kind() == io::ErrorKind::NotFound }
    pub fn forget(name: &str) { if let Ok(p) = marker(name) { let _ = std::fs::remove_file(p); } }
    pub fn hello_ok(name: &str, key: &[u8]) -> bool { !key.is_empty() && marker(name).and_then(std::fs::read).is_ok_and(|k| k == key) }

    pub fn names() -> Vec<String> {
        let mut v: Vec<String> = dir().and_then(std::fs::read_dir).into_iter().flatten().flatten()
            .filter_map(|e| e.file_name().to_str()?.strip_suffix(".pipe").map(String::from)).filter(|n| exo_core::session::valid_name(n)).collect();
        v.sort();
        v
    }

    /// SECURITY_ATTRIBUTES whose DACL grants this user's SID, and nobody else, full access.
    fn only_me() -> io::Result<SECURITY_ATTRIBUTES> {
        let err = || Err(io::Error::last_os_error());
        unsafe {
            let mut tok = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok) == 0 { return err() }
            let mut len = 0u32;
            GetTokenInformation(tok, TokenUser, null_mut(), 0, &mut len);
            let mut buf = vec![0u64; (len as usize).div_ceil(8)];   // u64: TOKEN_USER holds pointers
            let ok = GetTokenInformation(tok, TokenUser, buf.as_mut_ptr().cast(), len, &mut len);
            CloseHandle(tok);
            if ok == 0 { return err() }
            let mut s = null_mut();
            if ConvertSidToStringSidW((*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid, &mut s) == 0 { return err() }
            let sid = String::from_utf16_lossy(std::slice::from_raw_parts(s, (0..).take_while(|i| *s.add(*i) != 0).count()));
            LocalFree(s.cast());
            let mut sd = null_mut();
            // P: protected, nothing inherited. The descriptor lives as long as the keeper, so it is never freed.
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(wide(&format!("D:P(A;;GA;;;{sid})")).as_ptr(), SDDL_REVISION_1, &mut sd, null_mut()) == 0 { return err() }
            Ok(SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 })
        }
    }

    fn instance(pipe: &[u16], sa: &SECURITY_ATTRIBUTES, first: bool) -> io::Result<Pipe> {
        let h = unsafe {
            CreateNamedPipeW(pipe.as_ptr(), PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 },
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, PIPE_UNLIMITED_INSTANCES, 65536, 65536, 0, sa)
        };
        if h == INVALID_HANDLE_VALUE { Err(io::Error::last_os_error()) } else { Ok(Pipe(h)) }
    }

    pub struct Listener { pipe: Vec<u16>, sa: SECURITY_ATTRIBUTES, next: Mutex<Pipe>, key: Vec<u8>, marker: PathBuf }
    // SAFETY: the security descriptor behind `sa` is never freed or changed
    unsafe impl Send for Listener {}
    unsafe impl Sync for Listener {}

    impl Listener {
        /// Claims session `name`: the first instance of its pipe (AlreadyExists if a keeper has it), then the key.
        pub fn bind(name: &str) -> io::Result<Self> {
            let (pipe, sa) = (pipe_name(name), only_me()?);
            let first = instance(&pipe, &sa, true).map_err(|e| if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) { io::ErrorKind::AlreadyExists.into() } else { e })?;
            let mut key = [0u8; 16];
            ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut key).map_err(|_| io::Error::other("no randomness"))?;
            let key: Vec<u8> = key.iter().flat_map(|b| format!("{b:02x}").into_bytes()).collect();
            let marker = marker(name)?;
            std::fs::write(&marker, &key)?;
            Ok(Listener { pipe, sa, next: Mutex::new(first), key, marker })
        }
        pub fn accept(&self) -> io::Result<Conn> {
            let mut next = self.next.lock().unwrap();
            let h = next.0;
            match wait(h, |ov| unsafe { ConnectNamedPipe(h, ov) }) {
                Err(e) if e.raw_os_error() != Some(ERROR_PIPE_CONNECTED as i32) => return Err(e),
                _ => {}
            }
            let fresh = instance(&self.pipe, &self.sa, false)?;
            Ok(Conn(Arc::new(std::mem::replace(&mut *next, fresh))))
        }
        pub fn key(&self) -> Vec<u8> { self.key.clone() }
        pub fn remove(&self) { let _ = std::fs::remove_file(&self.marker); }
    }

    /// The caller's own stdin/stdout/stderr are not to be inherited by the keeper: the pane would not see the
    /// caller's output end while the keeper held a copy.
    pub fn before_spawn() {
        for s in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] { unsafe { SetHandleInformation(GetStdHandle(s), HANDLE_FLAG_INHERIT, 0) }; }
    }

    pub fn detach(c: &mut std::process::Command) {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x8;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
}
