//! The plugged-in FIDO2 security key, spoken to directly (CTAPHID over Linux hidraw): whether it wants a new PIN
//! before anything else (forcePINChange, set when the key is handed out), and changing the PIN. Just enough CTAP 2.1
//! for those two: authenticatorGetInfo and clientPIN changePIN with PIN protocol 1.
// ponytail: Linux only (hidraw); macOS needs IOKit HID and Windows its HID API, add when a holder uses one.

use std::fs::File;
use std::io::{Read, Write};

const INIT: u8 = 0x86;
const CBOR: u8 = 0x90;
const KEEPALIVE: u8 = 0xBB;
const ERROR: u8 = 0xBF;

/// Whether any FIDO security key is plugged in (its hidraw device is there, whoever may open it).
pub fn present() -> bool {
    std::fs::read_dir("/sys/class/hidraw").into_iter().flatten().flatten()
        .any(|e| std::fs::read(e.path().join("device/report_descriptor")).is_ok_and(|rd| rd.starts_with(&[0x06, 0xD0, 0xF1])))
}

/// The first hidraw device whose report descriptor opens with the FIDO usage page (0xF1D0) and that this user may open.
fn device() -> Result<File, String> {
    let mut seen = false;
    for e in std::fs::read_dir("/sys/class/hidraw").map_err(|e| e.to_string())?.flatten() {
        let rd = std::fs::read(e.path().join("device/report_descriptor")).unwrap_or_default();
        if !rd.starts_with(&[0x06, 0xD0, 0xF1]) { continue }
        seen = true;
        if let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).open(std::path::Path::new("/dev").join(e.file_name())) { return Ok(f) }
    }
    Err(if seen { "This computer does not let this account use the key (no access to its hidraw device).".into() } else { "Plug in your key.".into() })
}

struct Hid { f: File, cid: [u8; 4] }

impl Hid {
    fn open() -> Result<Hid, String> {
        let mut h = Hid { f: device()?, cid: [0xFF; 4] };
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
            self.f.write_all(&r).map_err(io)?;
            first = false;
        }
        let mut p = [0u8; 64];
        loop {
            self.f.read_exact(&mut p).map_err(io)?;
            if p[..4] != self.cid || p[4] == KEEPALIVE { continue }
            break;
        }
        if p[4] == ERROR { return Err(format!("The key refused the request (error {:#04x}).", p[7])) }
        let len = u16::from_be_bytes([p[5], p[6]]) as usize;
        let mut out = p[7..].to_vec();
        while out.len() < len { self.f.read_exact(&mut p).map_err(io)?; if p[..4] == self.cid { out.extend_from_slice(&p[5..]) } }
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

/// The key's light, by a vendor command of ExoPulse's pico-fido build (0x70): 1 unlocked (green, then green breathing),
/// 0 an unlock failed (red blinks), 2 locked (red blinks, then white breathing). Other keys answer with an error, which is ignored.
pub fn light(state: u8) { if let Ok(mut h) = Hid::open() { let _ = h.call(0xF0, &[state]); } }

/// Whether the key must get a new PIN before it is used (forcePINChange in authenticatorGetInfo).
pub fn must_change_pin() -> Result<bool, String> {
    let info = Hid::open()?.ctap(0x04, None).map_err(|s| format!("The key did not describe itself (status {s:#04x})."))?;
    Ok(matches!(info.get(0x0C), Some(Value::Bool(true))))
}

/// clientPIN changePIN, PIN protocol 1: a shared secret with the key by ECDH P-256, the new PIN and the old PIN's
/// hash sent encrypted with it.
pub fn change_pin(old: &str, new: &str) -> Result<(), String> {
    use ring::{digest, hmac};
    if new.chars().count() < 4 || new.len() > 63 { return Err("The new PIN needs 4 to 63 characters.".into()) }
    let mut h = Hid::open()?;
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

/// Sets up a key that has just been flashed: the PIN it is handed out with (when it has none yet), and
/// forcePINChange, so its holder must choose their own PIN before anything else. Some(message) when the key already
/// had a PIN (flashing keeps the key's data), which is kept.
pub fn hand_out(pin: &str) -> Result<Option<String>, String> {
    use ring::{digest, hmac};
    let mut h = Hid::open()?;
    let info = h.ctap(0x04, None).map_err(refused)?;
    let has_pin = matches!(info.get(4).and_then(|o| { let Value::Map(m) = o else { return None }; m.iter().find(|(k, _)| *k == Value::Text("clientPin".into())).map(|(_, v)| v) }), Some(Value::Bool(true)));
    if has_pin { return Ok(Some("This key already had a PIN, which was kept: flashing keeps a key's data. Reset the key to start it over.".into())) }
    // setPIN
    let (shared, platform_key) = agree(&mut h).map_err(|e| e.unwrap_or_else(|| "The key agreement failed.".into()))?;
    let mut padded = pin.as_bytes().to_vec();
    padded.resize(64, 0);
    let pin_enc = aes256_cbc_zero_iv(&shared, &padded);
    let tag = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, &shared), &pin_enc);
    h.ctap(0x06, Some(&enc_map(&[(1, Cb::U(1)), (2, Cb::U(3)), (3, Cb::Raw(&platform_key)), (4, Cb::B(&tag.as_ref()[..16])), (5, Cb::B(&pin_enc))]))).map_err(refused)?;
    // a token for authenticatorConfig (permission acfg, 0x20), then setMinPINLength with forceChangePin
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
    Ok(None)
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

#[derive(Debug, PartialEq)]
enum Value { Int(i64), Bytes(Vec<u8>), Text(String), Array(Vec<Value>), Map(Vec<(Value, Value)>), Bool(bool), Other }

impl Value {
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
