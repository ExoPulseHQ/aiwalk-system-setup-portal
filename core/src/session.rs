//! The OS-free half of `aiwalk-setup session` (desktop/src/sessioncli.rs): session names, the framed protocol between
//! a client and the keeper that holds the terminal, and what the keeper remembers of the terminal's output: a
//! bounded ring of raw bytes to replay to a pane that attaches, the last window title, and the modes the program
//! switched on.

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Read};

/// A session name becomes part of a socket path or pipe name, so only [A-Za-z0-9._-]{1,64}, and not "." or "..".
pub fn valid_name(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s != "." && s != ".." && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

// ---------------------------------------------------------------- protocol

/// Frame kinds. A frame is: kind (1 byte), body length (u32, little endian), body.
/// Client to keeper, as the connection's first frame: ATTACH (body: size), PASTE (body: text), END, LIST.
/// After ATTACH the client sends INPUT (bytes for the terminal) and RESIZE (size), any number, until it leaves.
/// Keeper to client: HELLO first on every connection (body: the Windows pipe key, empty elsewhere), then OUTPUT
/// (terminal bytes) and finally EXIT (i32 LE, the command's exit code) to an attached client, or INFO (JSON) for LIST.
/// A size is cols u16 LE, rows u16 LE.
pub mod kind {
    pub const ATTACH: u8 = b'A';
    pub const INPUT: u8 = b'I';
    pub const RESIZE: u8 = b'R';
    pub const PASTE: u8 = b'P';
    pub const END: u8 = b'E';
    pub const LIST: u8 = b'L';
    pub const HELLO: u8 = b'H';
    pub const OUTPUT: u8 = b'O';
    pub const EXIT: u8 = b'X';
    pub const INFO: u8 = b'J';
}

/// Larger bodies are refused: no frame is legitimately this big, and a garbage length must not allocate gigabytes.
const MAX_BODY: usize = 16 << 20;

pub fn frame(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(5 + body.len());
    f.push(kind);
    f.extend_from_slice(&(body.len() as u32).to_le_bytes());
    f.extend_from_slice(body);
    f
}

/// The next frame, None when the stream ended cleanly between frames.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<(u8, Vec<u8>)>> {
    let mut head = [0u8; 5];
    match r.read_exact(&mut head[..1]) {
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        x => x?,
    }
    r.read_exact(&mut head[1..])?;
    let len = u32::from_le_bytes(head[1..].try_into().unwrap()) as usize;
    if len > MAX_BODY { return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large")) }
    let mut body = vec![0; len];
    r.read_exact(&mut body)?;
    Ok(Some((head[0], body)))
}

pub fn size_body(cols: u16, rows: u16) -> [u8; 4] {
    let (c, r) = (cols.to_le_bytes(), rows.to_le_bytes());
    [c[0], c[1], r[0], r[1]]
}

pub fn parse_size(b: &[u8]) -> Option<(u16, u16)> {
    let (c, r) = (u16::from_le_bytes(b.get(0..2)?.try_into().ok()?), u16::from_le_bytes(b.get(2..4)?.try_into().ok()?));
    (b.len() == 4 && c > 0 && r > 0).then_some((c, r))
}

/// `text` as a bracketed paste: the program sees it as pasted, so a newline in it does not submit. Any paste-end
/// marker inside the text is removed first, or the text could end the paste early and type the rest as keys.
pub fn bracketed_paste(text: &[u8]) -> Vec<u8> {
    let mut t = text.to_vec();
    while let Some(i) = t.windows(6).position(|w| w == b"\x1b[201~") { t.drain(i..i + 6); }
    [&b"\x1b[200~"[..], &t, b"\x1b[201~"].concat()
}

// ---------------------------------------------------------------- terminal output

/// What a finished control sequence meant, as far as the keeper cares.
#[derive(Debug, PartialEq)]
pub enum Event {
    /// OSC 0 or OSC 2: the window title.
    Title(String),
    /// CSI ? Pn h / l: a DEC private mode switched on or off.
    Mode(u16, bool),
    /// A question to the terminal (device attributes, cursor position, colour queries ...). Replayed to a new pane it
    /// would be answered again, and the answer typed into the program as if the person had, so replay drops it.
    Query,
}

#[derive(Clone, Copy, Default, PartialEq, Debug)]
enum St { #[default] Ground, Utf8(u8), Esc, EscInter, Csi, Osc, OscEsc, Str, StrEsc }

/// A byte-at-a-time parser of terminal output, just deep enough to know where sequences start and end, so it may be
/// fed a stream split anywhere. Strings end with BEL or ST (ESC \); CAN and SUB abort a sequence as terminals do.
#[derive(Clone, Default)]
pub struct Vt { st: St, buf: Vec<u8> }

impl Vt {
    /// Between characters and sequences: a safe place to start showing the stream from.
    pub fn at_boundary(&self) -> bool { self.st == St::Ground }

    pub fn feed(&mut self, b: u8, ev: &mut Vec<Event>) {
        if b == 0x18 || b == 0x1a { return self.st = St::Ground }
        match self.st {
            St::Osc | St::OscEsc | St::Str | St::StrEsc => {}
            _ if b == 0x1b => { self.buf.clear(); return self.st = St::Esc }
            _ => {}
        }
        self.st = match self.st {
            St::Ground => match b { 0xc2..=0xdf => St::Utf8(1), 0xe0..=0xef => St::Utf8(2), 0xf0..=0xf4 => St::Utf8(3), _ => St::Ground },
            St::Utf8(n) => match b {
                0x80..=0xbf => if n == 1 { St::Ground } else { St::Utf8(n - 1) },
                _ => { self.st = St::Ground; return self.feed(b, ev) }   // broken character: look at this byte afresh
            },
            St::Esc => match b { b'[' => St::Csi, b']' => St::Osc, b'P' | b'X' | b'^' | b'_' => St::Str, 0x20..=0x2f => St::EscInter, _ => St::Ground },
            St::EscInter => if (0x20..=0x2f).contains(&b) { St::EscInter } else { St::Ground },
            St::Csi => match b {
                0x40..=0x7e => { self.csi(b, ev); St::Ground }
                0x20..=0x3f => { if self.buf.len() < 64 { self.buf.push(b) } St::Csi }
                _ => St::Csi,   // a control character inside CSI acts in place
            },
            St::Osc => match b {
                0x07 => { self.osc(ev); St::Ground }
                0x1b => St::OscEsc,
                _ => { if self.buf.len() < 4096 { self.buf.push(b) } St::Osc }
            },
            St::OscEsc | St::StrEsc if b == b'\\' => {
                if self.st == St::OscEsc { self.osc(ev) } else if self.buf == b"$q" || self.buf == b"+q" { ev.push(Event::Query) }
                St::Ground
            }
            // ESC not followed by \ inside a string: the string is over and a new sequence begins
            St::OscEsc | St::StrEsc => { self.buf.clear(); self.st = St::Esc; return self.feed(b, ev) }
            St::Str => if b == 0x1b { St::StrEsc } else { if self.buf.len() < 2 { self.buf.push(b) } St::Str },
        };
    }

    fn csi(&mut self, fin: u8, ev: &mut Vec<Event>) {
        let p = &self.buf;
        let first = p.split(|c| *c == b';').next().and_then(|n| std::str::from_utf8(n).ok()?.parse::<u16>().ok());
        if p.first() == Some(&b'?') && matches!(fin, b'h' | b'l') {
            for n in p[1..].split(|c| *c == b';') {
                if let Some(n) = std::str::from_utf8(n).ok().and_then(|s| s.parse().ok()) { ev.push(Event::Mode(n, fin == b'h')) }
            }
        }
        // device attributes, status and cursor position reports, kitty keyboard flags, XTVERSION, mode reports,
        // window size reports
        let query = match fin {
            b'c' | b'n' => true,
            b'u' => p.first() == Some(&b'?'),
            b'q' => p.first() == Some(&b'>'),
            b'p' => p.contains(&b'$'),
            b't' => matches!(first, Some(11 | 13 | 14 | 15 | 16 | 18 | 19 | 20 | 21)),
            _ => false,
        };
        if query { ev.push(Event::Query) }
    }

    fn osc(&mut self, ev: &mut Vec<Event>) {
        let b = std::mem::take(&mut self.buf);
        if let Some(t) = b.strip_prefix(b"0;").or_else(|| b.strip_prefix(b"2;")) {
            ev.push(Event::Title(String::from_utf8_lossy(t).into_owned()));
        } else if b.ends_with(b";?") {
            ev.push(Event::Query);   // colour queries (OSC 4, 10, 11, 12 ...)
        }
    }
}

/// The keeper's memory of the terminal: the last `cap` bytes of output, the title, and the private modes on.
pub struct Screen {
    buf: VecDeque<u8>,
    cap: usize,
    /// the parser's state at buf's first byte: it has seen everything that was cut off
    front: Vt,
    live: Vt,
    pub title: String,
    modes: BTreeMap<u16, bool>,
}

impl Screen {
    pub fn new(cap: usize) -> Self {
        Screen { buf: VecDeque::new(), cap, front: Vt::default(), live: Vt::default(), title: String::new(), modes: BTreeMap::new() }
    }

    pub fn is_empty(&self) -> bool { self.buf.is_empty() }

    pub fn push(&mut self, chunk: &[u8]) {
        let mut ev = vec![];
        for &b in chunk { self.live.feed(b, &mut ev) }
        for e in ev {
            match e {
                Event::Title(t) => self.title = t,
                // bounded: a program switching hundreds of distinct modes is not one to replay faithfully
                Event::Mode(m, on) if self.modes.len() < 64 || self.modes.contains_key(&m) => { self.modes.insert(m, on); }
                _ => {}
            }
        }
        self.buf.extend(chunk);
        if self.buf.len() > self.cap {
            let mut ev = vec![];
            for b in self.buf.drain(..self.buf.len() - self.cap) { self.front.feed(b, &mut ev); ev.clear() }
        }
    }

    /// What a newly attached pane is sent: the modes still on, then the kept output from the first point where no
    /// character or sequence is cut in half, with questions to the terminal left out.
    pub fn replay(&self) -> Vec<u8> {
        let mut out: Vec<u8> = self.modes.iter().filter(|(_, on)| **on).flat_map(|(m, _)| format!("\x1b[?{m}h").into_bytes()).collect();
        let (mut vt, mut ev, mut seq) = (self.front.clone(), vec![], 0);
        let mut started = vt.at_boundary();
        for &b in &self.buf {
            let was = vt.at_boundary();
            vt.feed(b, &mut ev);
            if !started { started = vt.at_boundary(); continue }
            if was { seq = out.len() }
            out.push(b);
            if ev.contains(&Event::Query) { out.truncate(seq) }
            ev.clear();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_that_are_safe_in_a_path_only() {
        for ok in ["t1", "claude-main", "A.b_c-9", &"x".repeat(64)] { assert!(valid_name(ok), "{ok}") }
        for bad in ["", ".", "..", "a/b", "a\\b", "a b", "ä", "a:b", &"x".repeat(65), "a\0"] { assert!(!valid_name(bad), "{bad:?}") }
    }

    #[test]
    fn frames_round_trip_and_garbage_is_refused() {
        let mut s = [frame(kind::ATTACH, &size_body(120, 40)), frame(kind::INPUT, b""), frame(kind::OUTPUT, &[7; 70000])].concat();
        let mut r = &s[..];
        assert_eq!(read_frame(&mut r).unwrap(), Some((kind::ATTACH, size_body(120, 40).to_vec())));
        assert_eq!(parse_size(&size_body(120, 40)), Some((120, 40)));
        assert_eq!(read_frame(&mut r).unwrap(), Some((kind::INPUT, vec![])));
        assert_eq!(read_frame(&mut r).unwrap().unwrap().1.len(), 70000);
        assert_eq!(read_frame(&mut r).unwrap(), None);
        s.truncate(3);
        assert!(read_frame(&mut &s[..]).is_err());   // cut inside a frame is an error, not a clean end
        assert!(read_frame(&mut &[b'O', 255, 255, 255, 255][..]).is_err());
        assert_eq!(parse_size(&size_body(0, 40)), None);
    }

    #[test]
    fn a_paste_cannot_end_itself_early() {
        assert_eq!(bracketed_paste(b"ls\n"), b"\x1b[200~ls\n\x1b[201~");
        assert_eq!(bracketed_paste(b"a\x1b[20\x1b[201~1~b"), b"\x1b[200~ab\x1b[201~");   // removal must not leave a new one
    }

    fn events(chunks: &[&[u8]]) -> Vec<Event> {
        let (mut vt, mut ev) = (Vt::default(), vec![]);
        for c in chunks { for &b in *c { vt.feed(b, &mut ev) } }
        ev
    }

    #[test]
    fn titles_by_bel_or_st_split_anywhere() {
        assert_eq!(events(&[b"x\x1b]0;hello\x07y"]), [Event::Title("hello".into())]);
        assert_eq!(events(&[b"\x1b]2;a b\x1b\\"]), [Event::Title("a b".into())]);
        let whole = b"ab\x1b]0;caf\xc3\xa9 \xe2\x9c\x93\x1b\\cd";
        for cut in 0..whole.len() { assert_eq!(events(&[&whole[..cut], &whole[cut..]]), [Event::Title("café ✓".into())], "cut at {cut}") }
        assert_eq!(events(&[b"\x1b]1;icon\x07\x1b]7;file:///x\x07"]), []);   // icon name, cwd: not the title
        assert_eq!(events(&[b"\x1b]0;one\x18\x1b]0;two\x07"]), [Event::Title("two".into())]);   // CAN aborts
        assert_eq!(events(&[b"\x1bP1$r\x07]0;in\x07\x1b\\"]), []);   // inside DCS nothing counts until ST
    }

    #[test]
    fn modes_and_queries() {
        assert_eq!(events(&[b"\x1b[?1049h\x1b[?1000;1006h\x1b[?25l\x1b[2J"]),
            [Event::Mode(1049, true), Event::Mode(1000, true), Event::Mode(1006, true), Event::Mode(25, false)]);
        assert_eq!(events(&[b"\x1b[c\x1b[>0c\x1b[6n\x1b[?u\x1b[>q\x1b[?2004$p\x1b]11;?\x07\x1b[18t\x1bP+q544e\x1b\\"]), (0..9).map(|_| Event::Query).collect::<Vec<_>>());
        assert_eq!(events(&[b"\x1b[!p\x1b[1;31m\x1b[3;4H\x1b[>1u\x1b[8;24;80t\x1b[2 q"]), []);   // soft reset, colours, cursor moves
    }

    #[test]
    fn replay_starts_whole_and_keeps_modes() {
        let mut s = Screen::new(1024);
        s.push(b"\x1b[?2004h\x1b]0;t\x07hello\x1b[?25l");
        assert_eq!(s.title, "t");
        assert_eq!(s.replay(), b"\x1b[?2004h\x1b[?2004h\x1b]0;t\x07hello\x1b[?25l");
        // once the cut falls inside a colour sequence, replay begins after it; mode 2004 is still restored
        let mut s = Screen::new(10);
        s.push(b"\x1b[?2004h0123456789\x1b[1;31mX\xc3\xa9Z\n");
        assert_eq!(s.replay(), b"\x1b[?2004hX\xc3\xa9Z\n");
        // ... or inside a two-byte character
        let mut s = Screen::new(2);
        s.push(b"ab\xc3");
        s.push(b"\xa9\n");
        assert_eq!(s.replay(), b"\n");
        // a question in the kept output is not asked again
        let mut s = Screen::new(1024);
        s.push(b"a\x1b[6nb\x1b[cc");
        assert_eq!(s.replay(), b"abc");
        assert!(Screen::new(8).replay().is_empty());
    }
}
