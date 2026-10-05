//! `aiwalk-setup deck pdf <source.html> [options]`: the vault's scripts/html-deck-to-pdf.py without Python or
//! Playwright. It starts the Chrome, Chromium or Edge already on the computer headless, with a throwaway profile, and
//! speaks the Chrome DevTools Protocol to it over a WebSocket: size the viewport, open the deck, scroll each
//! `<section class="slide" id="…">` into view, click through `--expand` targets, screenshot. The frames become one
//! PDF (exo_core::deck). Screenshots of the live page, rather than `chrome --print-to-pdf`, because the print engine
//! clips flex and grid content and cannot open tabs or `<details>`.
//!
//! The browser runs in its own process group (Unix) so the whole tree is killed at the end, and its profile folder
//! is removed, on success, on error and on Ctrl-C (which is caught and acted on after the browser's current answer).

use exo_core::deck::{self, Image, Plan};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket};

/// How long one DevTools answer (a page load, a screenshot) may take before the browser counts as not answering.
const ANSWER: Duration = Duration::from_secs(60);

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

pub fn main(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{}", deck::USAGE);
        return 0;
    }
    if args.first().map(String::as_str) != Some("pdf") {
        eprintln!("{}", deck::USAGE);
        return 1;
    }
    match deck::parse_args(&args[1..]).and_then(|plan| run(&plan)) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// `~/x` and relative paths made absolute, as Python's expanduser().resolve() does for a file that may not exist yet.
fn absolute(p: &str) -> PathBuf {
    let p = match p.strip_prefix("~/").or(p.strip_prefix("~\\")) { Some(rest) => crate::home().join(rest), None if p == "~" => crate::home(), None => PathBuf::from(p) };
    let p = std::env::current_dir().map(|d| d.join(&p)).unwrap_or(p);
    p.canonicalize().unwrap_or(p)
}

fn run(plan: &Plan) -> Result<(), String> {
    let src = absolute(&plan.source);
    if !src.is_file() { return Err(format!("source not found: {}", src.display())) }
    let out = plan.out.as_deref().map(absolute).unwrap_or_else(|| src.with_extension("pdf"));
    let exe = find_browser()?;
    catch_ctrl_c();

    let mut b = Browser::launch(&exe)?;
    b.open(&deck::file_url(&src.to_string_lossy()), plan)?;
    let slide_ids: Vec<String> = match &plan.slides {
        Some(s) => s.clone(),
        None => serde_json::from_value(b.eval("Array.from(document.querySelectorAll('section.slide[id]')).map(s => s.id)")?).unwrap_or_default(),
    };
    if slide_ids.is_empty() {
        return Err("no slides found — pass --slides explicitly, or check that <section class='slide' id='...'> exists".into());
    }
    // a typo in --slides or --expand stops before anything is rendered (the Python died mid-run on --slides, ignored --expand)
    let present: Vec<String> = serde_json::from_value(b.eval("Array.from(document.querySelectorAll('[id]')).map(e => e.id)")?).unwrap_or_default();
    if let Some(bad) = slide_ids.iter().chain(plan.expand.iter().map(|(s, _)| s)).find(|s| !present.contains(s)) {
        return Err(format!("slide id not in the deck: {bad}"));
    }

    let wait = Duration::from_millis(plan.wait_ms);
    let mut frames: Vec<Image> = Vec::new();
    for sid in &slide_ids {
        b.eval(&format!("document.getElementById({}).scrollIntoView({{block:'start', behavior:'instant'}})", js(sid)))?;
        std::thread::sleep(wait);
        match plan.expander(sid) {
            None => frames.push(b.shoot(sid)?),
            Some(sel) => {
                let n = b.eval(&format!("document.querySelectorAll({}).length", js(sel)))?.as_u64().unwrap_or(0);
                if n == 0 {
                    println!("  ! expander for {sid}: selector matched 0 elements; falling back to single frame");
                    frames.push(b.shoot(sid)?);
                }
                for i in 0..n {
                    b.eval(&format!("document.querySelectorAll({})[{i}].click()", js(sel)))?;
                    std::thread::sleep(wait);
                    frames.push(b.shoot(&format!("{sid}_t{}", i + 1))?);
                }
            }
        }
    }
    drop(b);

    println!("assembling {} frames → {}", frames.len(), out.display());
    let pdf = deck::pdf(&frames);
    if let Some(dir) = out.parent() { std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))? }
    std::fs::write(&out, &pdf).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("[ok] wrote {} ({:.1} KB)", out.display(), pdf.len() as f64 / 1024.0);
    Ok(())
}

/// A JavaScript string literal (JSON's is one).
fn js(s: &str) -> String { serde_json::to_string(s).unwrap() }

/// AIWALK_BROWSER when set (and then only it), else the first candidate for this OS that exists.
fn find_browser() -> Result<PathBuf, String> {
    let found = |c: &str| if c.contains('/') || c.contains('\\') { Some(PathBuf::from(c)).filter(|p| p.is_file()) } else { crate::on_path(c) };
    if let Some(b) = std::env::var("AIWALK_BROWSER").ok().filter(|b| !b.is_empty()) {
        return found(&b).ok_or(format!("no browser found: AIWALK_BROWSER={b} is not a file or a command on PATH"));
    }
    let candidates = deck::browser_candidates(std::env::consts::OS, &crate::home().to_string_lossy(), &|k| std::env::var(k).ok());
    candidates.iter().find_map(|c| found(c))
        .ok_or(format!("no browser found; looked for {} (set AIWALK_BROWSER to a Chrome, Chromium or Edge executable)", candidates.join(", ")))
}

/// Ctrl-C sets a flag the DevTools loop checks, so the browser is closed and its profile removed before exiting.
/// ponytail: Unix only; on Windows Ctrl-C ends this process at once and may leave the temp profile behind.
fn catch_ctrl_c() {
    #[cfg(unix)]
    {
        extern "C" fn on_int(_: libc::c_int) { INTERRUPTED.store(true, Ordering::SeqCst) }
        // SAFETY: the handler only stores to an atomic, which is async-signal-safe
        unsafe { libc::signal(libc::SIGINT, on_int as *const () as libc::sighandler_t) };
    }
}

/// A headless browser on a throwaway profile, attached to one page. Dropping it ends the browser and removes the profile.
struct Browser {
    child: Child,
    profile: PathBuf,
    ws: Option<WebSocket<TcpStream>>,
    session: String,
    next_id: u64,
    /// Event names seen for the page while waiting for answers, for wait_for.
    events: Vec<String>,
}

impl Browser {
    fn launch(exe: &Path) -> Result<Browser, String> {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
        let profile = std::env::temp_dir().join(format!("aiwalk-deck-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&profile).map_err(|e| format!("{}: {e}", profile.display()))?;
        let mut cmd = Command::new(exe);
        cmd.arg("--headless=new").arg(format!("--user-data-dir={}", profile.display()))
            .args(["--remote-debugging-port=0", "--hide-scrollbars", "--no-first-run", "--no-default-browser-check", "--disable-gpu", "--mute-audio", "about:blank"])
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
            // Chrome refuses to start its sandbox as root (a container, WSL): the Python always passed --no-sandbox
            if unsafe { libc::geteuid() } == 0 { cmd.arg("--no-sandbox"); }
        }
        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&profile);
                return Err(format!("{}: {e}", exe.display()));
            }
        };
        let mut b = Browser { child, profile, ws: None, session: String::new(), next_id: 0, events: vec![] };

        // Chrome prints "DevTools listening on ws://127.0.0.1:PORT/devtools/browser/ID" on stderr. Reading stderr
        // rather than <profile>/DevToolsActivePort also works for a snap or flatpak browser, whose /tmp is its own.
        let stderr = b.child.stderr.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
        std::thread::spawn(move || {
            let mut last = String::new();
            let mut sent = false;
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Some(url) = line.split("DevTools listening on ").nth(1).filter(|_| !sent) {
                    sent = tx.send(Ok(url.trim().to_string())).is_ok();
                } else if !line.trim().is_empty() {
                    last = line;
                }
            } // drained to the end so the browser never blocks on a full pipe
            let _ = tx.send(Err(last));
        });
        let url = match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok(url)) => url,
            Ok(Err(last)) => return Err(format!("the browser did not answer: {} exited ({})", exe.display(), if last.is_empty() { "no message" } else { last.trim() })),
            Err(_) => return Err(format!("the browser did not answer: {} gave no DevTools address in 30 s", exe.display())),
        };
        let addr = url.strip_prefix("ws://").and_then(|r| r.split('/').next()).ok_or(format!("the browser did not answer: odd DevTools address {url}"))?;
        let stream = TcpStream::connect(addr).map_err(|e| format!("the browser did not answer: {addr}: {e}"))?;
        stream.set_read_timeout(Some(ANSWER)).ok();
        // a 2560x1440 screenshot is several MB of base64 in one message
        let cfg = tungstenite::protocol::WebSocketConfig::default().max_message_size(Some(512 << 20)).max_frame_size(Some(512 << 20));
        let (ws, _) = tungstenite::client::client_with_config(url.as_str(), stream, Some(cfg)).map_err(|e| format!("the browser did not answer: {e}"))?;
        b.ws = Some(ws);
        Ok(b)
    }

    /// One DevTools command and its result. Events that arrive meanwhile are noted for wait_for.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        if INTERRUPTED.load(Ordering::SeqCst) { return Err("interrupted".into()) }
        self.next_id += 1;
        let id = self.next_id;
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if !self.session.is_empty() { msg["sessionId"] = json!(self.session) }
        let ws = self.ws.as_mut().unwrap();
        ws.send(Message::text(msg.to_string())).map_err(|e| format!("the browser did not answer ({method}): {e}"))?;
        loop {
            let v = self.read(method)?;
            if v["id"].as_u64() == Some(id) {
                return match v.get("error") {
                    Some(e) => Err(format!("{method}: {}", e["message"].as_str().unwrap_or("failed"))),
                    None => Ok(v["result"].clone()),
                };
            }
        }
    }

    /// The next message from the browser, noting it when it is an event.
    fn read(&mut self, waiting_for: &str) -> Result<Value, String> {
        loop {
            match self.ws.as_mut().unwrap().read() {
                Ok(Message::Text(t)) => {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap_or_default();
                    if let Some(m) = v["method"].as_str() { self.events.push(m.to_string()) }
                    return Ok(v);
                }
                Ok(_) => {}
                Err(_) if INTERRUPTED.load(Ordering::SeqCst) => return Err("interrupted".into()),
                Err(e) => return Err(format!("the browser did not answer ({waiting_for}): {e}")),
            }
        }
    }

    /// Waits until the page has sent `event` (at most ANSWER).
    fn wait_for(&mut self, event: &str) -> Result<(), String> {
        let start = Instant::now();
        while !self.events.iter().any(|e| e == event) {
            if start.elapsed() > ANSWER { return Err(format!("the browser did not answer ({event})")) }
            self.read(event)?;
        }
        Ok(())
    }

    /// A page of the plan's viewport and scale, showing `url` with its web fonts loaded.
    fn open(&mut self, url: &str, plan: &Plan) -> Result<(), String> {
        let target = self.call("Target.createTarget", json!({ "url": "about:blank" }))?;
        let attached = self.call("Target.attachToTarget", json!({ "targetId": target["targetId"], "flatten": true }))?;
        self.session = attached["sessionId"].as_str().unwrap_or_default().to_string();
        self.call("Page.enable", json!({}))?;
        self.call("Emulation.setDeviceMetricsOverride", json!({ "width": plan.width, "height": plan.height, "deviceScaleFactor": plan.dsf, "mobile": false }))?;
        self.events.clear();
        let nav = self.call("Page.navigate", json!({ "url": url }))?;
        if let Some(e) = nav["errorText"].as_str() { return Err(format!("the browser could not open {url}: {e}")) }
        self.wait_for("Page.loadEventFired")?;
        self.eval("document.fonts.ready.then(() => true)")?; // the Python waited for network idle; fonts are what show
        Ok(())
    }

    /// Runs `expr` in the page (awaiting a promise) and returns its value; a thrown error is an Err.
    fn eval(&mut self, expr: &str) -> Result<Value, String> {
        let r = self.call("Runtime.evaluate", json!({ "expression": expr, "returnByValue": true, "awaitPromise": true }))?;
        if let Some(ex) = r.get("exceptionDetails") {
            let why = ex["exception"]["description"].as_str().or(ex["text"].as_str()).unwrap_or("error");
            return Err(format!("{expr}: {}", why.lines().next().unwrap_or(why)));
        }
        Ok(r["result"]["value"].clone())
    }

    /// A screenshot of the viewport, at the device scale, ready for the PDF.
    fn shoot(&mut self, name: &str) -> Result<Image, String> {
        use base64::Engine;
        let r = self.call("Page.captureScreenshot", json!({ "format": "png" }))?;
        let png = base64::engine::general_purpose::STANDARD.decode(r["data"].as_str().unwrap_or_default()).map_err(|e| format!("screenshot {name}: {e}"))?;
        let frame = deck::png_to_rgb(&png).map_err(|e| format!("screenshot {name}: {e}"))?;
        println!("  captured {name}");
        Ok(frame.into())
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        // ask politely, then end the whole process tree (renderers, GPU process) and remove the profile
        if let Some(ws) = self.ws.as_mut() {
            let _ = ws.send(Message::text(json!({ "id": 0, "method": "Browser.close" }).to_string()));
        }
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) && matches!(self.child.try_wait(), Ok(None)) { std::thread::sleep(Duration::from_millis(50)) }
        #[cfg(unix)]
        unsafe { libc::kill(-(self.child.id() as i32), libc::SIGKILL); }
        let _ = self.child.kill();
        let _ = self.child.wait();
        // a child process may still be letting go of a file (Windows): retry for a moment
        for _ in 0..20 {
            if std::fs::remove_dir_all(&self.profile).is_ok() || !self.profile.exists() { break }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
