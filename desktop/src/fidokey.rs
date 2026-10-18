#![cfg_attr(windows, allow(dead_code))]
//! The plugged-in FIDO2 security key, spoken to directly in CTAP 2.1 over CTAPHID: Linux through hidraw, macOS through
//! IOKit (hidapi). Windows lets only its own WebAuthn service talk to security keys, so there the two operations sealed
//! notes need (a credential, and its hmac-secret) go through webauthn.dll (winauthn.rs) and Windows asks the PIN and the
//! touch itself; the PIN-management calls say to use Windows Settings.
//!
//! What is here: authenticatorGetInfo, clientPIN (PIN protocol 1: setPIN, changePIN, getPinToken, a token with
//! permissions), authenticatorConfig forcePINChange, makeCredential and getAssertion with hmac-secret, and the light
//! of ExoPulse's pico-fido build.

#[cfg(not(windows))]
use std::io::{Read, Write};

#[cfg(not(windows))]
const INIT: u8 = 0x86;
#[cfg(not(windows))]
const CBOR: u8 = 0x90;
#[cfg(not(windows))]
const KEEPALIVE: u8 = 0xBB;
#[cfg(not(windows))]
const ERROR: u8 = 0xBF;

/// A plugged-in FIDO security key: how this system names its device, the name it gives, and its kind ("yubikey",
/// "board" for the team's pico-fido boards, or "other").
#[derive(serde::Serialize, Clone)]
pub struct Dev { pub id: String, pub name: String, pub kind: String }

fn kind_of(name: &str) -> String {
    if name.contains("Yubico") || name.contains("YubiKey") { "yubikey".into() } else if name.contains("Pico Key") { "board".into() } else { "other".into() }
}

/// Every FIDO security key plugged in. Linux: hidraw devices whose report descriptor opens with the FIDO usage page
/// (0xF1D0).
#[cfg(target_os = "linux")]
pub fn devices() -> Vec<Dev> {
    let mut v: Vec<Dev> = std::fs::read_dir("/sys/class/hidraw").into_iter().flatten().flatten()
        .filter(|e| std::fs::read(e.path().join("device/report_descriptor")).is_ok_and(|rd| rd.starts_with(&[0x06, 0xD0, 0xF1])))
        .map(|e| {
            let uevent = std::fs::read_to_string(e.path().join("device/uevent")).unwrap_or_default();
            let name = uevent.lines().find_map(|l| l.strip_prefix("HID_NAME=")).unwrap_or("Security key").to_string();
            Dev { id: e.file_name().to_string_lossy().into_owned(), kind: kind_of(&name), name }
        }).collect();
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
}

/// macOS and Windows: HID devices on the FIDO usage page, as hidapi lists them (listing needs no access; on Windows
/// opening one does, which is why the operations go through WebAuthn there).
#[cfg(any(target_os = "macos", windows))]
pub fn devices() -> Vec<Dev> {
    let Ok(api) = hidapi::HidApi::new() else { return vec![] };
    let mut v: Vec<Dev> = api.device_list().filter(|d| d.usage_page() == 0xF1D0 && d.usage() == 0x01).map(|d| {
        let name = format!("{} {}", d.manufacturer_string().unwrap_or_default(), d.product_string().unwrap_or("Security key")).trim().to_string();
        Dev { id: d.path().to_string_lossy().into_owned(), kind: kind_of(&name), name }
    }).collect();
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v.dedup_by(|a, b| a.id == b.id);
    v
}


/// The id of the key to talk to: `id` when given, else the only one plugged in.
#[cfg(not(windows))]
fn chosen(id: Option<&str>) -> Result<String, String> {
    let all = devices();
    match id {
        Some(id) => all.iter().find(|d| d.id == id).map(|d| d.id.clone()).ok_or("That key is not plugged in any more.".into()),
        None => match all.as_slice() {
            [] => Err("Plug in your key.".into()),
            [one] => Ok(one.id.clone()),
            _ => Err("More than one key is plugged in: choose which one.".into()),
        },
    }
}

/// The open device: 65-byte reports out (report id 0 first), 64-byte reports in.
#[cfg(target_os = "linux")]
struct Port(std::fs::File);
#[cfg(target_os = "linux")]
impl Port {
    fn open(id: &str) -> Result<Port, String> {
        std::fs::OpenOptions::new().read(true).write(true).open(std::path::Path::new("/dev").join(id)).map(Port)
            .map_err(|_| "This computer does not let this account use the key (no access to its hidraw device).".into())
    }
    fn send(&mut self, r: &[u8; 65]) -> std::io::Result<()> { self.0.write_all(r) }
    fn recv(&mut self, p: &mut [u8; 64]) -> std::io::Result<()> { self.0.read_exact(p) }
}
#[cfg(target_os = "macos")]
struct Port(hidapi::HidDevice);
#[cfg(target_os = "macos")]
impl Port {
    fn open(id: &str) -> Result<Port, String> {
        let api = hidapi::HidApi::new().map_err(|e| e.to_string())?;
        let path = std::ffi::CString::new(id).map_err(|e| e.to_string())?;
        api.open_path(&path).map(Port).map_err(|e| format!("The key could not be opened: {e}"))
    }
    fn send(&mut self, r: &[u8; 65]) -> std::io::Result<()> { self.0.write(r).map(|_| ()).map_err(std::io::Error::other) }
    fn recv(&mut self, p: &mut [u8; 64]) -> std::io::Result<()> {
        match self.0.read(p) { Ok(64) => Ok(()), Ok(_) => Err(std::io::Error::other("short report")), Err(e) => Err(std::io::Error::other(e)) }
    }
}

#[cfg(not(windows))]
struct Hid { f: Port, cid: [u8; 4] }

#[cfg(not(windows))]
impl Hid {
    fn open(id: Option<&str>) -> Result<Hid, String> {
        let mut h = Hid { f: Port::open(&chosen(id)?)?, cid: [0xFF; 4] };
        let nonce: [u8; 8] = std::array::from_fn(|i| (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos() >> (i * 3)) as u8);
        let r = h.call(INIT, &nonce)?;
        if r.len() < 12 || r[..8] != nonce { return Err("The key gave an unexpected answer.".into()) }
        h.cid.copy_from_slice(&r[8..12]);
        Ok(h)
    }

    /// One CTAPHID transaction: the request in 64-byte reports, the answer read back (keep-alives skipped).
    fn call(&mut self, cmd: u8, data: &[u8]) -> Result<Vec<u8>, String> {
        let io = |e: std::io::Error| format!("The key could not be reached: {e}");
        let mut first = true;
        let (mut rest, mut seq) = (data, 0u8);
        while first || !rest.is_empty() {
            let mut r = [0u8; 65];   // report id 0, then the 64-byte packet
            r[1..5].copy_from_slice(&self.cid);
            let room = if first { r[5] = cmd; r[6..8].copy_from_slice(&(data.len() as u16).to_be_bytes()); 57 } else { r[5] = seq; seq += 1; 59 };
            let n = room.min(rest.len());
            let at = 65 - room;
            r[at..at + n].copy_from_slice(&rest[..n]);
            rest = &rest[n..];
            self.f.send(&r).map_err(io)?;
            first = false;
        }
        let mut p = [0u8; 64];
        loop {
            self.f.recv(&mut p).map_err(io)?;
            if p[..4] != self.cid || p[4] == KEEPALIVE { continue }
            break;
        }
        if p[4] == ERROR { return Err(format!("The key refused the request (error {:#04x}).", p[7])) }
        let len = u16::from_be_bytes([p[5], p[6]]) as usize;
        let mut out = p[7..].to_vec();
        while out.len() < len { self.f.recv(&mut p).map_err(io)?; if p[..4] == self.cid { out.extend_from_slice(&p[5..]) } }
        out.truncate(len);
        Ok(out)
    }

    /// An authenticator command: the status byte checked, the CBOR answer decoded.
    fn ctap(&mut self, cmd: u8, params: Option<&[u8]>) -> Result<Value, u8> {
        let mut req = vec![cmd];
        req.extend_from_slice(params.unwrap_or_default());
        let r = self.call(CBOR, &req).map_err(|_| 0xFF)?;
        match r.split_first() { Some((0, body)) if body.is_empty() => Ok(Value::Map(vec![])), Some((0, body)) => Ok(decode(&mut &body[..]).ok_or(0xFE)?), Some((&s, _)) => Err(s), None => Err(0xFF) }
    }
}

#[cfg(not(windows))]
/// The key's light, by a vendor command of ExoPulse's pico-fido build (0x70): 1 unlocked (green, then green breathing),
/// 0 an unlock failed (red blinks), 2 locked (red blinks, then white breathing). Other keys answer with an error, which is ignored.
pub fn light(id: Option<&str>, state: u8) { if let Ok(mut h) = Hid::open(id) { let _ = h.call(0xF0, &[state]); } }

#[cfg(not(windows))]
/// Whether the key must get a new PIN before it is used (forcePINChange in authenticatorGetInfo).
pub fn must_change_pin(id: Option<&str>) -> Result<bool, String> {
    let info = Hid::open(id)?.ctap(0x04, None).map_err(|s| format!("The key did not describe itself (status {s:#04x})."))?;
    Ok(matches!(info.get(0x0C), Some(Value::Bool(true))))
}

#[cfg(not(windows))]
/// clientPIN changePIN, PIN protocol 1: a shared secret with the key by ECDH P-256, the new PIN and the old PIN's
/// hash sent encrypted with it.
pub fn change_pin(id: Option<&str>, old: &str, new: &str) -> Result<(), String> {
    use ring::{digest, hmac};
    if new.chars().count() < 4 || new.len() > 63 { return Err("The new PIN needs 4 to 63 characters.".into()) }
    let mut h = Hid::open(id)?;
    let (shared, platform_key) = agree(&mut h).map_err(|e| e.unwrap_or_else(|| "The key agreement failed.".into()))?;
    let mut pin = new.as_bytes().to_vec();
    pin.resize(64, 0);
    let new_enc = aes256_cbc_zero_iv(&shared, &pin);
    let old_hash = digest::digest(&digest::SHA256, old.as_bytes());
    let old_enc = aes256_cbc_zero_iv(&shared, &old_hash.as_ref()[..16]);
    let tag = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, &shared), &[&new_enc[..], &old_enc[..]].concat());
    let req = enc_map(&[(1, Cb::U(1)), (2, Cb::U(4)), (3, Cb::Raw(&platform_key)), (4, Cb::B(&tag.as_ref()[..16])), (5, Cb::B(&new_enc)), (6, Cb::B(&old_enc))]);
    h.ctap(0x06, Some(&req)).map(|_| ()).map_err(refused)
}

fn refused(s: u8) -> String {
    match s {
        0x31 => "The current PIN is wrong.".into(),
        0x32 | 0x34 => "The key is locked after too many wrong PINs; unplug it and plug it in again (it may need a reset).".into(),
        0x37 => "The key does not accept this new PIN (too short, or the same as before).".into(),
        s => format!("The key refused (status {s:#04x})."),
    }
}

#[cfg(not(windows))]
/// PIN protocol 1's shared secret with the key (ECDH P-256, SHA-256 of x) and this side's COSE key to send with it.
/// Err(Some) is the key's refusal, Err(None) a failure here.
fn agree(h: &mut Hid) -> Result<([u8; 32], Vec<u8>), Option<String>> {
    use ring::{agreement, digest, rand::SystemRandom};
    let ka = h.ctap(0x06, Some(&enc_map(&[(1, Cb::U(1)), (2, Cb::U(2))]))).map_err(|s| Some(refused(s)))?;
    let cose = ka.get(1).ok_or(None)?;
    let (Some(Value::Bytes(x)), Some(Value::Bytes(y))) = (cose.get(-2), cose.get(-3)) else { return Err(None) };
    let rng = SystemRandom::new();
    let mine = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).map_err(|_| None)?;
    let p = mine.compute_public_key().map_err(|_| None)?;
    let platform_key = enc_map(&[(1, Cb::U(2)), (3, Cb::N(-25)), (-1, Cb::U(1)), (-2, Cb::B(&p.as_ref()[1..33])), (-3, Cb::B(&p.as_ref()[33..65]))]);
    let theirs = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, [&[4u8][..], x, y].concat());
    let shared = agreement::agree_ephemeral(mine, &theirs, |z| -> [u8; 32] { digest::digest(&digest::SHA256, z).as_ref().try_into().unwrap() }).map_err(|_| None)?;
    Ok((shared, platform_key))
}

#[cfg(not(windows))]
/// Whether the plugged-in key has a PIN yet.
pub fn has_pin(id: Option<&str>) -> Result<bool, String> {
    let info = Hid::open(id)?.ctap(0x04, None).map_err(refused)?;
    Ok(matches!(info.get(4).and_then(|o| { let Value::Map(m) = o else { return None }; m.iter().find(|(k, _)| *k == Value::Text("clientPin".into())).map(|(_, v)| v) }), Some(Value::Bool(true))))
}

#[cfg(not(windows))]
/// Gives a key that has just been flashed the PIN it is handed out with; false when it already had a PIN (flashing
/// keeps a key's data), which is then kept.
pub fn set_pin_if_none(id: Option<&str>, pin: &str) -> Result<bool, String> {
    use ring::hmac;
    let mut h = Hid::open(id)?;
    let info = h.ctap(0x04, None).map_err(refused)?;
    let has_pin = matches!(info.get(4).and_then(|o| { let Value::Map(m) = o else { return None }; m.iter().find(|(k, _)| *k == Value::Text("clientPin".into())).map(|(_, v)| v) }), Some(Value::Bool(true)));
    if has_pin { return Ok(false) }
    let (shared, platform_key) = agree(&mut h).map_err(|e| e.unwrap_or_else(|| "The key agreement failed.".into()))?;
    let mut padded = pin.as_bytes().to_vec();
    padded.resize(64, 0);
    let pin_enc = aes256_cbc_zero_iv(&shared, &padded);
    let tag = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, &shared), &pin_enc);
    h.ctap(0x06, Some(&enc_map(&[(1, Cb::U(1)), (2, Cb::U(3)), (3, Cb::Raw(&platform_key)), (4, Cb::B(&tag.as_ref()[..16])), (5, Cb::B(&pin_enc))]))).map_err(refused)?;
    Ok(true)
}

#[cfg(not(windows))]
/// forcePINChange: the key refuses to work until its holder chose their own PIN. A token with the acfg permission
/// (0x20), then authenticatorConfig setMinPINLength with forceChangePin.
pub fn force_pin_change(id: Option<&str>, pin: &str) -> Result<(), String> {
    use ring::{digest, hmac};
    let mut h = Hid::open(id)?;
    let (shared, platform_key) = agree(&mut h).map_err(|e| e.unwrap_or_else(|| "The key agreement failed.".into()))?;
    let hash = digest::digest(&digest::SHA256, pin.as_bytes());
    let hash_enc = aes256_cbc_zero_iv(&shared, &hash.as_ref()[..16]);
    let r = h.ctap(0x06, Some(&enc_map(&[(1, Cb::U(1)), (2, Cb::U(9)), (3, Cb::Raw(&platform_key)), (6, Cb::B(&hash_enc)), (9, Cb::U(0x20))]))).map_err(refused)?;
    let Some(Value::Bytes(token_enc)) = r.get(2) else { return Err("The key gave no token.".into()) };
    let token = aes256_cbc_zero_iv_decrypt(&shared, token_enc);
    let params = enc_map(&[(3, Cb::T)]);
    let mut msg = vec![0xFF; 32];
    msg.extend([0x0D, 0x03]);
    msg.extend_from_slice(&params);
    let tag = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, &token), &msg);
    h.ctap(0x0D, Some(&enc_map(&[(1, Cb::U(3)), (2, Cb::Raw(&params)), (3, Cb::U(1)), (4, Cb::B(&tag.as_ref()[..16]))]))).map_err(refused)?;
    Ok(())
}

/// The relying party age-plugin-fido2-hmac uses: credentials made here and by the plugin are the same kind.
pub const RP_ID: &str = "age-encryption.org";

fn random<const N: usize>() -> [u8; N] { let mut b = [0u8; N]; ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b).expect("random bytes"); b }

#[cfg(not(windows))]
/// A PIN token (clientPIN getPinToken, protocol 1): it authorises makeCredential and getAssertion with user verification.
fn pin_token(h: &mut Hid, pin: &str) -> Result<Vec<u8>, String> {
    let (shared, platform_key) = agree(h).map_err(|e| e.unwrap_or_else(|| "The key agreement failed.".into()))?;
    let hash = ring::digest::digest(&ring::digest::SHA256, pin.as_bytes());
    let hash_enc = aes256_cbc_zero_iv(&shared, &hash.as_ref()[..16]);
    let r = h.ctap(0x06, Some(&enc_map(&[(1, Cb::U(1)), (2, Cb::U(5)), (3, Cb::Raw(&platform_key)), (6, Cb::B(&hash_enc))]))).map_err(refused)?;
    let Some(Value::Bytes(t)) = r.get(2) else { return Err("The key gave no PIN token.".into()) };
    Ok(aes256_cbc_zero_iv_decrypt(&shared, t))
}

fn pin_auth(token: &[u8], msg: &[u8]) -> Vec<u8> { ring::hmac::sign(&ring::hmac::Key::new(ring::hmac::HMAC_SHA256, token), msg).as_ref()[..16].to_vec() }

#[cfg(not(windows))]
/// makeCredential for sealed notes: not discoverable, with the hmac-secret extension, PIN-verified. Its credential ID.
pub fn make_credential(id: Option<&str>, pin: &str) -> Result<Vec<u8>, String> {
    let mut h = Hid::open(id)?;
    let token = pin_token(&mut h, pin)?;
    let cdh: [u8; 32] = random();
    let user: [u8; 16] = random();
    let req = c_map(vec![
        (c_int(1), c_bytes(&cdh)),
        (c_int(2), c_map(vec![(c_text("id"), c_text(RP_ID)), (c_text("name"), c_text(RP_ID))])),
        (c_int(3), c_map(vec![(c_text("id"), c_bytes(&user)), (c_text("name"), c_text("age"))])),
        (c_int(4), c_arr(vec![c_map(vec![(c_text("alg"), c_int(-7)), (c_text("type"), c_text("public-key"))])])),
        (c_int(6), c_map(vec![(c_text("hmac-secret"), vec![0xF5])])),
        (c_int(8), c_bytes(&pin_auth(&token, &cdh))),
        (c_int(9), c_int(1)),
    ]);
    let r = h.ctap(0x01, Some(&req)).map_err(|s| if s == 0x2F || s == 0x27 { "The key was not touched in time.".into() } else { refused(s) })?;
    let Some(Value::Bytes(ad)) = r.get(2) else { return Err("The key gave no credential.".into()) };
    // authData: rpIdHash 32, flags 1, signCount 4, AAGUID 16, credential ID length 2, credential ID
    if ad.len() < 55 { return Err("The key gave a short credential.".into()) }
    let n = u16::from_be_bytes([ad[53], ad[54]]) as usize;
    ad.get(55..55 + n).map(<[u8]>::to_vec).ok_or("The key gave a malformed credential.".into())
}

/// Why getAssertion gave no secret: the credential is another key's, or the person did not act.
pub enum Miss { NotThisKey, Failed(String) }

#[cfg(not(windows))]
/// getAssertion with the hmac-secret extension for one credential and salt: the 32-byte secret that key derives. With
/// `pin` the assertion is user-verified, which is a different secret (the credential's "with UV" one).
pub fn hmac_secret(id: Option<&str>, pin: Option<&str>, cred: &[u8], salt: &[u8; 32]) -> Result<[u8; 32], Miss> {
    let fail = |e: String| Miss::Failed(e);
    let mut h = Hid::open(id).map_err(fail)?;
    let token = match pin { Some(p) => Some(pin_token(&mut h, p).map_err(fail)?), None => None };
    let (shared, platform_key) = agree(&mut h).map_err(|e| fail(e.unwrap_or_else(|| "The key agreement failed.".into())))?;
    let salt_enc = aes256_cbc_zero_iv(&shared, salt);
    let salt_auth = pin_auth(&shared, &salt_enc);
    let cdh: [u8; 32] = random();
    let mut pairs = vec![
        (c_int(1), c_text(RP_ID)),
        (c_int(2), c_bytes(&cdh)),
        (c_int(3), c_arr(vec![c_map(vec![(c_text("id"), c_bytes(cred)), (c_text("type"), c_text("public-key"))])])),
        (c_int(4), c_map(vec![(c_text("hmac-secret"), c_map(vec![(c_int(1), platform_key), (c_int(2), c_bytes(&salt_enc)), (c_int(3), c_bytes(&salt_auth))]))])),
    ];
    if let Some(t) = &token { pairs.push((c_int(6), c_bytes(&pin_auth(t, &cdh)))); pairs.push((c_int(7), c_int(1))); }
    let r = h.ctap(0x02, Some(&c_map(pairs))).map_err(|s| match s {
        0x2E => Miss::NotThisKey,
        0x2F | 0x27 => Miss::Failed("The key was not touched in time.".into()),
        s => Miss::Failed(refused(s)),
    })?;
    let Some(Value::Bytes(ad)) = r.get(2) else { return Err(fail("The key gave no assertion.".into())) };
    // authData: rpIdHash 32, flags 1 (0x80: extensions follow), signCount 4, then the extensions map
    if ad.len() < 38 || ad[32] & 0x80 == 0 { return Err(fail("The key gave no hmac-secret.".into())) }
    let ext = decode(&mut &ad[37..]).ok_or_else(|| fail("The key's extensions were unreadable.".into()))?;
    let Some(Value::Bytes(enc)) = ext.get_text("hmac-secret") else { return Err(fail("The key gave no hmac-secret.".into())) };
    aes256_cbc_zero_iv_decrypt(&shared, enc).get(..32).and_then(|b| b.try_into().ok()).ok_or_else(|| fail("The key's secret had the wrong length.".into()))
}

fn aes256_cbc_zero_iv_decrypt(key: &[u8; 32], data: &[u8]) -> Vec<u8> {
    use aes::cipher::{BlockCipherDecrypt, KeyInit};
    let c = aes::Aes256::new(key.into());
    let (mut prev, mut out) = ([0u8; 16], Vec::with_capacity(data.len()));
    for chunk in data.chunks(16) {
        let mut b: [u8; 16] = std::array::from_fn(|i| chunk[i]);
        let ct = b;
        c.decrypt_block((&mut b).into());
        out.extend((0..16).map(|i| b[i] ^ prev[i]));
        prev = ct;
    }
    out
}

fn aes256_cbc_zero_iv(key: &[u8; 32], data: &[u8]) -> Vec<u8> {
    use aes::cipher::{BlockCipherEncrypt, KeyInit};
    let c = aes::Aes256::new(key.into());
    let (mut prev, mut out) = ([0u8; 16], Vec::with_capacity(data.len()));
    for chunk in data.chunks(16) {
        let mut b: [u8; 16] = std::array::from_fn(|i| chunk[i] ^ prev[i]);
        c.encrypt_block((&mut b).into());
        out.extend_from_slice(&b);
        prev = b;
    }
    out
}

// ---------------------------------------------------------------- the little CBOR CTAP needs

enum Cb<'a> { U(u64), N(i64), B(&'a [u8]), Raw(&'a [u8]), T }

fn head(out: &mut Vec<u8>, major: u8, n: u64) {
    let m = major << 5;
    match n {
        0..=23 => out.push(m | n as u8),
        24..=0xFF => out.extend([m | 24, n as u8]),
        0x100..=0xFFFF => { out.push(m | 25); out.extend((n as u16).to_be_bytes()) }
        _ => { out.push(m | 26); out.extend((n as u32).to_be_bytes()) }
    }
}

fn int(out: &mut Vec<u8>, v: i64) { if v >= 0 { head(out, 0, v as u64) } else { head(out, 1, (-1 - v) as u64) } }

/// A map with integer keys, in the order given (CTAP's canonical order when the keys are listed that way).
fn enc_map(entries: &[(i64, Cb)]) -> Vec<u8> {
    let mut out = Vec::new();
    head(&mut out, 5, entries.len() as u64);
    for (k, v) in entries {
        int(&mut out, *k);
        match v { Cb::U(n) => head(&mut out, 0, *n), Cb::N(n) => int(&mut out, *n), Cb::B(b) => { head(&mut out, 2, b.len() as u64); out.extend_from_slice(b) } Cb::Raw(r) => out.extend_from_slice(r), Cb::T => out.push(0xF5) }
    }
    out
}

/// CBOR pieces built by hand, for the requests with text keys and nested maps (makeCredential, getAssertion). Map
/// entries are given in CTAP's canonical order (shorter keys first).
fn c_text(t: &str) -> Vec<u8> { let mut o = Vec::new(); head(&mut o, 3, t.len() as u64); o.extend_from_slice(t.as_bytes()); o }
fn c_bytes(b: &[u8]) -> Vec<u8> { let mut o = Vec::new(); head(&mut o, 2, b.len() as u64); o.extend_from_slice(b); o }
fn c_int(v: i64) -> Vec<u8> { let mut o = Vec::new(); int(&mut o, v); o }
fn c_arr(items: Vec<Vec<u8>>) -> Vec<u8> { let mut o = Vec::new(); head(&mut o, 4, items.len() as u64); items.into_iter().for_each(|i| o.extend(i)); o }
fn c_map(pairs: Vec<(Vec<u8>, Vec<u8>)>) -> Vec<u8> { let mut o = Vec::new(); head(&mut o, 5, pairs.len() as u64); pairs.into_iter().for_each(|(k, v)| { o.extend(k); o.extend(v) }); o }

#[derive(Debug, PartialEq)]
enum Value { Int(i64), Bytes(Vec<u8>), Text(String), Array(Vec<Value>), Map(Vec<(Value, Value)>), Bool(bool), Other }

impl Value {
    fn get_text(&self, key: &str) -> Option<&Value> {
        let Value::Map(m) = self else { return None };
        m.iter().find(|(k, _)| matches!(k, Value::Text(t) if t == key)).map(|(_, v)| v)
    }

    fn get(&self, key: i64) -> Option<&Value> {
        let Value::Map(m) = self else { return None };
        m.iter().find(|(k, _)| *k == Value::Int(key)).map(|(_, v)| v)
    }
}

fn decode(b: &mut &[u8]) -> Option<Value> {
    let (&first, rest) = b.split_first()?;
    *b = rest;
    let (major, info) = (first >> 5, first & 0x1F);
    let mut take = |n: usize| -> Option<&[u8]> { if b.len() < n { return None } let (h, t) = b.split_at(n); *b = t; Some(h) };
    let n: u64 = match info {
        0..=23 => info as u64,
        24 => take(1)?[0] as u64,
        25 => u16::from_be_bytes(take(2)?.try_into().ok()?) as u64,
        26 => u32::from_be_bytes(take(4)?.try_into().ok()?) as u64,
        27 => u64::from_be_bytes(take(8)?.try_into().ok()?),
        _ => return None,
    };
    Some(match major {
        0 => Value::Int(n as i64),
        1 => Value::Int(-1 - n as i64),
        2 => Value::Bytes(take(n as usize)?.to_vec()),
        3 => Value::Text(String::from_utf8_lossy(take(n as usize)?).into()),
        4 => Value::Array((0..n).map(|_| decode(b)).collect::<Option<_>>()?),
        5 => Value::Map((0..n).map(|_| Some((decode(b)?, decode(b)?))).collect::<Option<_>>()?),
        7 if info == 20 => Value::Bool(false),
        7 if info == 21 => Value::Bool(true),
        _ => Value::Other,
    })
}

// Windows: only WebAuthn may talk to security keys; Windows asks the PIN and the touch, and keeps PINs in its Settings.
#[cfg(windows)]
const SETTINGS: &str = "On Windows, the key's PIN is managed in Settings, Accounts, Sign-in options, Security key.";
#[cfg(windows)]
pub fn light(_: Option<&str>, _: u8) {}
#[cfg(windows)]
pub fn must_change_pin(_: Option<&str>) -> Result<bool, String> { Ok(false) }
#[cfg(windows)]
pub fn change_pin(_: Option<&str>, _: &str, _: &str) -> Result<(), String> { Err(SETTINGS.into()) }
#[cfg(windows)]
pub fn has_pin(_: Option<&str>) -> Result<bool, String> { Ok(true) }
#[cfg(windows)]
pub fn set_pin_if_none(_: Option<&str>, _: &str) -> Result<bool, String> { Ok(false) }
#[cfg(windows)]
pub fn force_pin_change(_: Option<&str>, _: &str) -> Result<(), String> { Err(SETTINGS.into()) }
#[cfg(windows)]
pub fn make_credential(_: Option<&str>, _: &str) -> Result<Vec<u8>, String> { crate::winauthn::make_credential() }
#[cfg(windows)]
pub fn hmac_secret(_: Option<&str>, pin: Option<&str>, cred: &[u8], salt: &[u8; 32]) -> Result<[u8; 32], Miss> { crate::winauthn::hmac_secret(pin.is_some(), cred, salt) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cbor_round_trip_and_aes() {
        let m = enc_map(&[(1, Cb::U(2)), (-2, Cb::B(&[9, 9])), (3, Cb::N(-25)), (12, Cb::U(300))]);
        let v = decode(&mut &m[..]).unwrap();
        assert_eq!(v.get(1), Some(&Value::Int(2)));
        assert_eq!(v.get(-2), Some(&Value::Bytes(vec![9, 9])));
        assert_eq!(v.get(3), Some(&Value::Int(-25)));
        assert_eq!(v.get(12), Some(&Value::Int(300)));
        assert_eq!(decode(&mut &[0xA1, 0x0C, 0xF5][..]).unwrap().get(0x0C), Some(&Value::Bool(true)));
        // FIPS-197 AES-256 vector: one block with a zero IV is plain AES
        let key: [u8; 32] = std::array::from_fn(|i| i as u8);
        let pt: [u8; 16] = std::array::from_fn(|i| (i as u8) * 0x11);
        let two: Vec<u8> = (0..32).collect();
        assert_eq!(aes256_cbc_zero_iv_decrypt(&key, &aes256_cbc_zero_iv(&key, &two)), two);
        assert_eq!(aes256_cbc_zero_iv(&key, &pt), [0x8e, 0xa2, 0xb7, 0xca, 0x51, 0x67, 0x45, 0xbf, 0xea, 0xfc, 0x49, 0x90, 0x4b, 0x49, 0x60, 0x89]);
    }
}
