//! The OS-free half of `aiwalk-setup deck pdf` (desktop/src/deckcli.rs), which renders a scroll-snap HTML deck to a
//! multi-page PDF by screenshotting each `<section class="slide" id="…">` in an installed Chrome, Chromium or Edge.
//! It replaces the vault's scripts/html-deck-to-pdf.py (Playwright + PIL). Here: the command line as a plan, where
//! to look for a browser on each OS, the deck's `file://` URL, PNG decoding and the PDF writer.
//!
//! The PDF keeps the Python's page geometry: one page per frame, the frame's pixels at 96 dpi, so a 1280x720 viewport
//! at device scale 2 is a 2560x1440 image on a 1920x1080 pt page. PIL stored frames as JPEG at quality 75; here they
//! are Flate-compressed RGB, lossless.

pub const USAGE: &str = "usage: aiwalk-setup deck pdf <source.html> [--out file.pdf] [--viewport 1280x720] [--dsf 2] [--wait 450]
                              [--slides s1,s2,...] [--expand \"slideID:cssSelector\"]...

  --out       output PDF (default: <source>.pdf, next to the source)
  --viewport  viewport WxH in CSS pixels (default 1280x720)
  --dsf       device scale factor (default 2: the PDF's images are viewport x dsf pixels)
  --wait      ms to wait after each scroll or click (default 450)
  --slides    comma-separated slide ids in order (default: every <section class=\"slide\" id=\"...\">)
  --expand    click each element matching cssSelector and take one frame per state, in place of slideID's
              single frame. Repeatable; the last one for a slide wins.

The browser is AIWALK_BROWSER if set, else the first Chrome, Chromium or Edge found on this computer.";

/// What `deck pdf` was asked to do.
#[derive(Debug, PartialEq)]
pub struct Plan {
    pub source: String,
    /// None: the source with its extension replaced by .pdf.
    pub out: Option<String>,
    pub width: u32,
    pub height: u32,
    pub dsf: f64,
    pub wait_ms: u64,
    /// None: every slide the deck has, in document order.
    pub slides: Option<Vec<String>>,
    /// (slide id, CSS selector), in the order given.
    pub expand: Vec<(String, String)>,
}

impl Plan {
    /// The selector to click through for this slide; like the Python's dict, the last --expand for a slide wins.
    pub fn expander(&self, slide: &str) -> Option<&str> {
        self.expand.iter().rev().find(|(s, _)| s == slide).map(|(_, sel)| sel.as_str())
    }
}

/// The arguments after `deck pdf`. Options take `--opt value` or `--opt=value`, as argparse does.
pub fn parse_args(args: &[String]) -> Result<Plan, String> {
    let mut p = Plan { source: String::new(), out: None, width: 1280, height: 720, dsf: 2.0, wait_ms: 450, slides: None, expand: vec![] };
    let mut source = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if !a.starts_with("--") {
            if source.replace(a.clone()).is_some() { return Err(format!("unexpected argument: {a}")) }
            continue;
        }
        let (name, inline) = match a.split_once('=') { Some((n, v)) => (n, Some(v.to_string())), None => (a.as_str(), None) };
        let value = match inline.or_else(|| it.next().cloned()) { Some(v) => v, None => return Err(format!("{name} needs a value")) };
        match name {
            "--out" => p.out = Some(value),
            "--viewport" => {
                let (w, h) = value.to_lowercase().split_once('x').and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)))
                    .filter(|&(w, h): &(u32, u32)| w > 0 && h > 0).ok_or(format!("--viewport must be WxH, e.g. 1280x720: {value}"))?;
                (p.width, p.height) = (w, h);
            }
            "--dsf" => p.dsf = value.parse().ok().filter(|d: &f64| d.is_finite() && *d > 0.0).ok_or(format!("--dsf must be a positive number: {value}"))?,
            "--wait" => p.wait_ms = value.parse().map_err(|_| format!("--wait must be milliseconds: {value}"))?,
            "--slides" => p.slides = Some(value.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()),
            "--expand" => {
                let (s, sel) = value.split_once(':').filter(|(s, sel)| !s.is_empty() && !sel.is_empty())
                    .ok_or(format!("--expand must be slideID:cssSelector: {value}"))?;
                p.expand.push((s.to_string(), sel.to_string()));
            }
            _ => return Err(format!("unknown option: {name}")),
        }
    }
    p.source = source.ok_or("missing <source.html>")?;
    Ok(p)
}

/// Where to look for a Chromium-family browser, in order. Linux entries without a slash are names to find on PATH.
/// `os` is std::env::consts::OS; `env` reads environment variables (Windows folders), `home` is the user's folder.
pub fn browser_candidates(os: &str, home: &str, env: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    match os {
        "macos" => {
            let apps = [("Google Chrome", "Google Chrome"), ("Chromium", "Chromium"), ("Microsoft Edge", "Microsoft Edge")];
            ["/Applications".to_string(), format!("{home}/Applications")].iter()
                .flat_map(|dir| apps.iter().map(move |(app, exe)| format!("{dir}/{app}.app/Contents/MacOS/{exe}"))).collect()
        }
        "windows" => {
            let exes = [r"Google\Chrome\Application\chrome.exe", r"Microsoft\Edge\Application\msedge.exe", r"Chromium\Application\chrome.exe"];
            ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"].iter().filter_map(|k| env(k))
                .flat_map(|dir| exes.iter().map(move |e| format!(r"{dir}\{e}"))).collect()
        }
        _ => {
            let mut c: Vec<String> = ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "microsoft-edge", "microsoft-edge-stable", "/snap/bin/chromium"]
                .map(String::from).into();
            for dir in ["/var/lib/flatpak/exports/bin".to_string(), format!("{home}/.local/share/flatpak/exports/bin")] {
                c.extend(["com.google.Chrome", "org.chromium.Chromium", "com.microsoft.Edge"].map(|app| format!("{dir}/{app}")));
            }
            c
        }
    }
}

/// `file://` URL of an absolute path, percent-encoding everything but unreserved characters and `/` (the Python did
/// not encode, so a `#` or `?` in a folder name broke it). Windows `C:\a b` becomes `file:///C:/a%20b`.
pub fn file_url(path: &str) -> String {
    let p = path.replace('\\', "/");
    let p = p.strip_prefix("//?/").unwrap_or(&p); // a canonicalized Windows path
    let mut url = String::from(if p.starts_with('/') { "file://" } else { "file:///" });
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:".contains(&b) { url.push(b as char) } else { url.push_str(&format!("%{b:02X}")) }
    }
    url
}

/// A frame as 8-bit RGB, row by row, no padding.
pub struct Rgb {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Decodes the PNG Chrome returns (8-bit RGB or RGBA, not interlaced) to RGB. Alpha is dropped, not composited, as
/// PIL's convert("RGB") does; a screenshot is opaque anyway.
pub fn png_to_rgb(png: &[u8]) -> Result<Rgb, String> {
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") { return Err("not a PNG".into()) }
    let (mut ihdr, mut idat, mut pos) = (None, Vec::new(), 8);
    while pos + 8 <= png.len() {
        let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
        let body = png.get(pos + 8..pos + 8 + len).ok_or("truncated PNG")?;
        match &png[pos + 4..pos + 8] {
            b"IHDR" => ihdr = Some(body.to_vec()),
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }
    let h = ihdr.filter(|h| h.len() == 13).ok_or("PNG without a header")?;
    let (width, height) = (u32::from_be_bytes(h[0..4].try_into().unwrap()), u32::from_be_bytes(h[4..8].try_into().unwrap()));
    let bpp = match (h[8], h[9], h[12]) {
        (8, 2, 0) => 3,
        (8, 6, 0) => 4,
        (d, c, i) => return Err(format!("unsupported PNG: bit depth {d}, colour type {c}, interlace {i}")),
    };
    let data = miniz_oxide::inflate::decompress_to_vec_zlib(&idat).map_err(|e| format!("PNG data: {e:?}"))?;
    let stride = width as usize * bpp;
    if data.len() != (stride + 1) * height as usize { return Err("PNG data has the wrong length".into()) }
    // undo the per-row filters (PNG spec section 9), then keep R, G, B
    let mut prev = vec![0u8; stride];
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 3);
    for row in data.chunks_exact(stride + 1) {
        let mut cur = row[1..].to_vec();
        for i in 0..stride {
            let a = if i >= bpp { cur[i - bpp] as i16 } else { 0 };
            let (b, c) = (prev[i] as i16, if i >= bpp { prev[i - bpp] as i16 } else { 0 });
            cur[i] = cur[i].wrapping_add(match row[0] {
                0 => 0,
                1 => a as u8,
                2 => b as u8,
                3 => ((a + b) / 2) as u8,
                4 => {
                    let p = a + b - c;
                    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                    (if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }) as u8
                }
                f => return Err(format!("bad PNG row filter {f}")),
            });
        }
        pixels.extend(cur.chunks_exact(bpp).flat_map(|px| &px[..3]));
        prev = cur;
    }
    Ok(Rgb { width, height, pixels })
}

/// A frame ready for the PDF: its RGB pixels zlib-compressed, so a long deck never holds more than one raw frame.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub zlib: Vec<u8>,
}

impl From<Rgb> for Image {
    fn from(f: Rgb) -> Image { Image { width: f.width, height: f.height, zlib: miniz_oxide::deflate::compress_to_vec_zlib(&f.pixels, 6) } }
}

/// A PDF with one page per frame, each page the frame's size at 96 dpi (pixels x 0.75 pt) and the frame drawn
/// over all of it as a Flate-compressed DeviceRGB image.
pub fn pdf(frames: &[Image]) -> Vec<u8> {
    let n = frames.len();
    // objects: 1 catalog, 2 page tree, then per frame: page, content stream, image
    let mut out = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::with_capacity(2 + 3 * n);
    let mut obj = |out: &mut Vec<u8>, head: String, stream: Option<&[u8]>| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{head}\n", offsets.len()).as_bytes());
        if let Some(s) = stream {
            out.extend_from_slice(b"stream\n");
            out.extend_from_slice(s);
            out.extend_from_slice(b"\nendstream\n");
        }
        out.extend_from_slice(b"endobj\n");
    };
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + 3 * i)).collect();
    obj(&mut out, "<< /Type /Catalog /Pages 2 0 R >>".into(), None);
    obj(&mut out, format!("<< /Type /Pages /Count {n} /Kids [{}] >>", kids.join(" ")), None);
    for (i, f) in frames.iter().enumerate() {
        let (content, image) = (4 + 3 * i, 5 + 3 * i);
        let (w, h) = (f.width as f64 * 0.75, f.height as f64 * 0.75);
        obj(&mut out, format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Resources << /XObject << /Im0 {image} 0 R >> >> /Contents {content} 0 R >>"), None);
        let draw = format!("q {w} 0 0 {h} 0 0 cm /Im0 Do Q");
        obj(&mut out, format!("<< /Length {} >>", draw.len()), Some(draw.as_bytes()));
        obj(&mut out, format!("<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>", f.width, f.height, f.zlib.len()), Some(&f.zlib));
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets { out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes()) }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> { s.iter().map(|a| a.to_string()).collect() }

    #[test]
    fn defaults_match_the_python() {
        let p = parse_args(&args(&["deck.html"])).unwrap();
        assert_eq!(p, Plan { source: "deck.html".into(), out: None, width: 1280, height: 720, dsf: 2.0, wait_ms: 450, slides: None, expand: vec![] });
    }

    #[test]
    fn options_in_both_spellings() {
        let p = parse_args(&args(&["--viewport=1920X1080", "d.html", "--dsf", "1.5", "--wait", "0", "--slides", " s2, s1 ,,", "--out", "x.pdf",
                                    "--expand", "s3:#t .tab-btn[data-tab]", "--expand=s3:a:hover", "--expand", "s4:details"])).unwrap();
        assert_eq!((p.width, p.height, p.dsf, p.wait_ms), (1920, 1080, 1.5, 0));
        assert_eq!(p.slides, Some(vec!["s2".to_string(), "s1".to_string()]));
        assert_eq!(p.out.as_deref(), Some("x.pdf"));
        assert_eq!(p.expander("s3"), Some("a:hover")); // the last one for a slide wins, and a ':' in the selector stays
        assert_eq!(p.expander("s4"), Some("details"));
        assert_eq!(p.expander("s1"), None);
    }

    #[test]
    fn bad_arguments_are_refused() {
        for bad in [&["--viewport", "1280"][..], &["d.html", "--viewport", "0x720"], &["d.html", "--dsf", "-1"], &["d.html", "--wait", "x"],
                    &["d.html", "--expand", "s3"], &["d.html", "--expand", ":sel"], &["d.html", "--keep"], &["d.html", "--out"], &["a.html", "b.html"], &[]] {
            assert!(parse_args(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn browsers_per_os() {
        let env = |k: &str| match k { "ProgramFiles" => Some(r"C:\Program Files".to_string()), "LOCALAPPDATA" => Some(r"C:\Users\a\AppData\Local".to_string()), _ => None };
        let w = browser_candidates("windows", "", &env);
        assert_eq!(w[0], r"C:\Program Files\Google\Chrome\Application\chrome.exe");
        assert!(w.contains(&r"C:\Users\a\AppData\Local\Microsoft\Edge\Application\msedge.exe".to_string()));
        assert_eq!(w.len(), 6); // ProgramFiles(x86) unset: skipped
        let m = browser_candidates("macos", "/Users/a", &env);
        assert_eq!(m[0], "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
        assert_eq!(m[5], "/Users/a/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge");
        let l = browser_candidates("linux", "/home/a", &env);
        assert_eq!(&l[..2], ["google-chrome", "google-chrome-stable"]);
        assert!(l.contains(&"/home/a/.local/share/flatpak/exports/bin/com.google.Chrome".to_string()));
    }

    #[test]
    fn file_urls_are_encoded() {
        assert_eq!(file_url("/a/b c/碩論#1.html"), "file:///a/b%20c/%E7%A2%A9%E8%AB%96%231.html");
        assert_eq!(file_url(r"\\?\C:\x y\d.html"), "file:///C:/x%20y/d.html");
    }

    /// A PNG of `rows` (each row: filter byte + pixels), for the decoder test.
    fn png(w: u32, h: u32, colour: u8, rows: &[u8]) -> Vec<u8> {
        let chunk = |kind: &[u8], body: &[u8]| [&(body.len() as u32).to_be_bytes()[..], kind, body, &[0; 4]].concat(); // CRC is not checked
        let ihdr = [&w.to_be_bytes()[..], &h.to_be_bytes(), &[8, colour, 0, 0, 0]].concat();
        let z = miniz_oxide::deflate::compress_to_vec_zlib(rows, 6);
        [&b"\x89PNG\r\n\x1a\n"[..], &chunk(b"IHDR", &ihdr), &chunk(b"IDAT", &z[..3]), &chunk(b"IDAT", &z[3..]), &chunk(b"IEND", b"")].concat()
    }

    #[test]
    fn png_filters_are_undone_and_alpha_dropped() {
        // 2x3 RGBA, one row per filter type that uses the row above or to the left
        let raw: [[u8; 8]; 3] = [[10, 20, 30, 255, 15, 25, 35, 255], [12, 22, 32, 255, 17, 27, 37, 255], [9, 8, 7, 255, 200, 100, 50, 255]];
        let mut rows = vec![1u8]; // Sub
        rows.extend((0..8).map(|i| raw[0][i].wrapping_sub(if i >= 4 { raw[0][i - 4] } else { 0 })));
        rows.push(2); // Up
        rows.extend((0..8).map(|i| raw[1][i].wrapping_sub(raw[0][i])));
        rows.push(4); // Paeth
        rows.extend((0..8).map(|i| {
            let (a, b, c) = (if i >= 4 { raw[2][i - 4] as i16 } else { 0 }, raw[1][i] as i16, if i >= 4 { raw[1][i - 4] as i16 } else { 0 });
            let p = a + b - c;
            let pred = if (p - a).abs() <= (p - b).abs() && (p - a).abs() <= (p - c).abs() { a } else if (p - b).abs() <= (p - c).abs() { b } else { c };
            raw[2][i].wrapping_sub(pred as u8)
        }));
        let f = png_to_rgb(&png(2, 3, 6, &rows)).unwrap();
        assert_eq!((f.width, f.height), (2, 3));
        assert_eq!(f.pixels, [10, 20, 30, 15, 25, 35, 12, 22, 32, 17, 27, 37, 9, 8, 7, 200, 100, 50]);
        // RGB with the Average filter
        let f = png_to_rgb(&png(1, 2, 2, &[0, 100, 50, 0, 3, 3, 27, 2])).unwrap();
        assert_eq!(f.pixels, [100, 50, 0, 53, 52, 2]);
        assert!(png_to_rgb(b"GIF89a").is_err());
        assert!(png_to_rgb(&png(1, 1, 0, &[0, 0])).is_err()); // greyscale
    }

    #[test]
    fn pdf_parses_back() {
        let small = vec![1, 2, 3, 4, 5, 6, 7, 8, 9];
        let frames = [Rgb { width: 2560, height: 1440, pixels: vec![200; 2560 * 1440 * 3] }, Rgb { width: 3, height: 1, pixels: small.clone() }];
        let doc = pdf(&frames.map(Image::from));
        let text: String = doc.iter().map(|&b| if b < 128 { b as char } else { '?' }).collect(); // byte offsets stay put
        assert!(text.starts_with("%PDF-1.4") && text.ends_with("%%EOF\n"));
        assert_eq!(text.matches(" 0 obj\n").count(), 2 + 3 * 2);
        assert_eq!(text.matches("/Type /Page ").count(), 2);
        assert!(text.contains("/Count 2 /Kids [3 0 R 6 0 R]"));
        assert!(text.contains("/MediaBox [0 0 1920 1080]") && text.contains("/MediaBox [0 0 2.25 0.75]"));
        // every xref offset points at its object, and startxref at the table
        let xref: usize = text.rsplit("startxref\n").next().unwrap().lines().next().unwrap().parse().unwrap();
        assert!(text[xref..].starts_with("xref\n0 9\n"));
        for (i, line) in text[xref..].lines().skip(3).take(8).enumerate() {
            let off: usize = line[..10].parse().unwrap();
            assert!(text[off..].starts_with(&format!("{} 0 obj", i + 1)), "object {}", i + 1);
        }
        // the image streams inflate back to the pixels
        let start = text.find("/Height 1 ").unwrap();
        let s = start + text[start..].find("stream\n").unwrap() + 7;
        let len: usize = text[start..].split("/Length ").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
        assert_eq!(miniz_oxide::inflate::decompress_to_vec_zlib(&doc[s..s + len]).unwrap(), small);
    }
}
