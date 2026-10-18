//! age with FIDO2 security keys, inside the app: the format of age-plugin-fido2-hmac (its spec v2), so the keys and files
//! made with the plugin keep working and nobody installs age or the plugin. Only the key talk is per operating system
//! (fidokey.rs).
//!
//! A key's recipient is `age1fido2-hmac1…`: version 2, an X25519 public key, a PIN flag, a salt and a credential ID.
//! The X25519 secret behind it is the hmac-secret the key derives from that credential and salt, so encrypting needs
//! only the recipient, and decrypting needs the key, its PIN and a touch.

use age::{DecryptError, EncryptError};
use age_core::format::{FileKey, Stanza, FILE_KEY_BYTES};
use age_core::primitives::{aead_decrypt, aead_encrypt, hkdf};
use base64::{prelude::BASE64_STANDARD_NO_PAD, Engine};
use secrecy::ExposeSecret;
use std::collections::HashSet;
use x25519_dalek::{EphemeralSecret, PublicKey, StaticSecret};

const HRP: &str = "age1fido2-hmac";
const TAG: &str = "fido2-hmac";
const LABEL: &[u8] = b"age-encryption.org/v1/X25519";
const VERSION: [u8; 2] = [0, 2];

/// A holder's key as a recipient.
#[derive(Clone, PartialEq, Debug)]
pub struct Recipient { pub pk: [u8; 32], pub pin: bool, pub salt: [u8; 32], pub cred: Vec<u8> }

impl Recipient {
    pub fn parse(s: &str) -> Option<Recipient> {
        let (hrp, data, _) = bech32::decode(s.trim()).ok()?;
        if hrp != HRP { return None }
        let b = <Vec<u8> as bech32::FromBase32>::from_base32(&data).ok()?;
        if b.len() < 2 + 32 + 1 + 32 + 1 || b[..2] != VERSION { return None }
        Some(Recipient { pk: b[2..34].try_into().ok()?, pin: b[34] == 1, salt: b[35..67].try_into().ok()?, cred: b[67..].to_vec() })
    }

    pub fn encode(&self) -> String {
        let mut b = VERSION.to_vec();
        b.extend_from_slice(&self.pk);
        b.push(self.pin as u8);
        b.extend_from_slice(&self.salt);
        b.extend_from_slice(&self.cred);
        bech32::encode(HRP, bech32::ToBase32::to_base32(&b), bech32::Variant::Bech32).expect("a valid prefix")
    }
}

/// X25519 wrapping as age does it, with the shared secret salted by both public keys.
fn wrap_key(pk: &PublicKey, epk: &PublicKey, shared: &[u8; 32]) -> [u8; 32] {
    let mut salt = [0u8; 64];
    salt[..32].copy_from_slice(epk.as_bytes());
    salt[32..].copy_from_slice(pk.as_bytes());
    hkdf(&salt, LABEL, shared)
}

impl age::Recipient for Recipient {
    fn wrap_file_key(&self, file_key: &FileKey) -> Result<(Vec<Stanza>, HashSet<String>), EncryptError> {
        let pk = PublicKey::from(self.pk);
        let esk = EphemeralSecret::random_from_rng(rand_core::OsRng);
        let epk = PublicKey::from(&esk);
        let shared = esk.diffie_hellman(&pk);
        let body = aead_encrypt(&wrap_key(&pk, &epk, shared.as_bytes()), file_key.expose_secret());
        let b = |x: &[u8]| BASE64_STANDARD_NO_PAD.encode(x);
        let args = vec![b(&VERSION), b(epk.as_bytes()), b(&[self.pin as u8]), b(&self.salt), b(&self.cred)];
        Ok((vec![Stanza { tag: TAG.into(), args, body }], HashSet::new()))
    }
}

/// What a plugin stanza names: the credential, salt and PIN flag to ask the key with, and the ephemeral share.
struct Named { epk: [u8; 32], pin: bool, salt: [u8; 32], cred: Vec<u8> }

fn named(st: &Stanza) -> Option<Named> {
    if st.tag != TAG || st.args.len() != 5 { return None }
    let d = |i: usize| BASE64_STANDARD_NO_PAD.decode(&st.args[i]).ok();
    if d(0)? != VERSION { return None }
    Some(Named { epk: d(1)?.try_into().ok()?, pin: d(2)? == [1], salt: d(3)?.try_into().ok()?, cred: d(4)? })
}

/// The plugged-in key as an age identity: for each plugin stanza it asks the key for that credential's secret (a key
/// that does not hold the credential says so at once, without a touch) and unwraps the file key with it.
pub struct Identity<'a> { pub dev: Option<&'a str>, pub pin: &'a str, pub why: std::cell::RefCell<Option<String>> }

/// The file key from a stanza, given the secret its key derived.
fn unwrap_with(secret: [u8; 32], n: &Named, st: &Stanza) -> Option<Result<FileKey, DecryptError>> {
    let sk = StaticSecret::from(secret);
    let (pk, epk) = (PublicKey::from(&sk), PublicKey::from(n.epk));
    let key = wrap_key(&pk, &epk, sk.diffie_hellman(&epk).as_bytes());
    aead_decrypt(&key, FILE_KEY_BYTES, &st.body).ok().map(|pt| Ok(FileKey::init_with_mut(|k| k.copy_from_slice(&pt))))
}

impl Identity<'_> {
    fn failed(&self, e: String) -> Option<Result<FileKey, DecryptError>> { *self.why.borrow_mut() = Some(e); Some(Err(DecryptError::KeyDecryptionFailed)) }
}

impl age::Identity for Identity<'_> {
    fn unwrap_stanza(&self, st: &Stanza) -> Option<Result<FileKey, DecryptError>> {
        let n = named(st)?;
        match crate::fidokey::hmac_secret(self.dev, n.pin.then_some(self.pin), &n.cred, &n.salt) {
            Ok(secret) => unwrap_with(secret, &n, st),
            Err(crate::fidokey::Miss::NotThisKey) => None,
            Err(crate::fidokey::Miss::Failed(e)) => self.failed(e),
        }
    }

    /// Windows asks through a prompt of its own: every holder's credential in one request, so the person sees one
    /// prompt, not one per stanza.
    #[cfg(windows)]
    fn unwrap_stanzas(&self, stanzas: &[Stanza]) -> Option<Result<FileKey, DecryptError>> {
        let ours: Vec<(&Stanza, Named)> = stanzas.iter().filter_map(|st| named(st).map(|n| (st, n))).collect();
        if ours.is_empty() { return None }
        let list: Vec<(Vec<u8>, [u8; 32])> = ours.iter().map(|(_, n)| (n.cred.clone(), n.salt)).collect();
        match crate::winauthn::hmac_secret_any(ours.iter().any(|(_, n)| n.pin), &list) {
            Ok((i, secret)) => unwrap_with(secret, &ours[i].1, ours[i].0),
            Err(crate::fidokey::Miss::NotThisKey) => None,
            Err(crate::fidokey::Miss::Failed(e)) => self.failed(e),
        }
    }
}

/// A new credential on the key, and its recipient: PIN required to decrypt, no separate identity (the plugin's choices
/// "1" and "2"). The key is touched twice: once to make the credential, once to derive its public key.
pub fn new_recipient(dev: Option<&str>, pin: &str) -> Result<String, String> {
    let cred = crate::fidokey::make_credential(dev, pin)?;
    let salt: [u8; 32] = { let mut b = [0u8; 32]; ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b).map_err(|_| "no random numbers")?; b };
    let secret = crate::fidokey::hmac_secret(dev, Some(pin), &cred, &salt).map_err(|m| match m {
        crate::fidokey::Miss::NotThisKey => "The key did not recognise the credential it just made.".to_string(),
        crate::fidokey::Miss::Failed(e) => e,
    })?;
    let pk = PublicKey::from(&StaticSecret::from(secret)).to_bytes();
    Ok(Recipient { pk, pin: true, salt, cred }.encode())
}

/// `text` encrypted to these recipients (keys' and plain age ones), ASCII-armoured as the vault keeps its files.
pub fn encrypt(recipients: &[Box<dyn age::Recipient>], text: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let enc = age::Encryptor::with_recipients(recipients.iter().map(|r| r.as_ref())).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let armor = age::armor::ArmoredWriter::wrap_output(&mut out, age::armor::Format::AsciiArmor).map_err(|e| e.to_string())?;
    let mut w = enc.wrap_output(armor).map_err(|e| e.to_string())?;
    w.write_all(text).map_err(|e| e.to_string())?;
    w.finish().and_then(|a| a.finish()).map_err(|e| e.to_string())?;
    Ok(out)
}

pub fn decrypt(identity: &dyn age::Identity, file: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let dec = age::Decryptor::new(age::armor::ArmoredReader::new(file)).map_err(|e| e.to_string())?;
    let mut r = dec.decrypt(std::iter::once(identity)).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    r.read_to_end(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

/// Every recipient line of `text` as a recipient: a key's (`age1fido2-hmac1…`) or a plain age one (`age1…`).
pub fn recipients(lines: &[String]) -> Result<Vec<Box<dyn age::Recipient>>, String> {
    lines.iter().map(|l| -> Result<Box<dyn age::Recipient>, String> {
        if let Some(r) = Recipient::parse(l) { return Ok(Box::new(r)) }
        l.parse::<age::x25519::Recipient>().map(|r| Box::new(r) as Box<dyn age::Recipient>).map_err(|_| format!("not a recipient: {l}"))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipient_round_trips_and_wraps_like_the_plugin() {
        let secret = [7u8; 32];
        let pk = PublicKey::from(&StaticSecret::from(secret)).to_bytes();
        let r = Recipient { pk, pin: true, salt: [9; 32], cred: vec![1, 2, 3, 4, 5] };
        let s = r.encode();
        assert!(s.starts_with("age1fido2-hmac1"));
        assert_eq!(Recipient::parse(&s), Some(r.clone()));
        // a stanza written for it opens with the secret the key would derive
        let fk = FileKey::new(Box::new([42u8; 16]));
        let (st, _) = age::Recipient::wrap_file_key(&r, &fk).unwrap();
        let n = named(&st[0]).unwrap();
        assert_eq!((n.pin, n.salt, n.cred.clone()), (true, [9; 32], vec![1, 2, 3, 4, 5]));
        let sk = StaticSecret::from(secret);
        let epk = PublicKey::from(n.epk);
        let key = wrap_key(&PublicKey::from(&sk), &epk, sk.diffie_hellman(&epk).as_bytes());
        assert_eq!(aead_decrypt(&key, FILE_KEY_BYTES, &st[0].body).unwrap(), vec![42u8; 16]);
    }

    #[test]
    fn notes_seal_and_open_for_the_group_key() {
        let id = age::x25519::Identity::generate();
        let sealed = encrypt(&recipients(&[id.to_public().to_string()]).unwrap(), b"# a note").unwrap();
        assert!(sealed.starts_with(b"-----BEGIN AGE ENCRYPTED FILE-----"));
        assert_eq!(decrypt(&id, &sealed).unwrap(), b"# a note");
    }

    #[test]
    fn parses_the_vaults_existing_recipients() {
        // the public key of the first holder's Pico, as the plugin wrote it on 2026-10-17
        let s = "age1fido2-hmac1qqpp6d6ztetysq8jzmucd3h80z6nlswcmvldp74uz5exc4dy8suaufcpax35kej6pw652hpp8wdxpa906m8l8j8musvf5yh0ew2x5ec200rlr5qzqkyfv6f2xm0zlea873me9s4afs5vjzuj2fhwe947m0rjkq27f32z6nwexq085j0ce93ywem3wutyxfkd2te337xufvq45flrmxjzh23gy9wdrkrdxdy252tnaapvczelkkvaehzrhn85ndqq2jn49rhcf3yxsxz63rqrgwynjpttwr7phgnw7wwgrw0yc2u0x4zhexpdhhqdw3hp48fuajyl7p23mhjm65jmugn4ds3v3k2ph485z9d53ryy2x0xw9vy36eqlnn63ru7j8nsak3y5dl55s80gn0";
        let r = Recipient::parse(s).expect("parses");
        assert!(r.pin);
        assert_eq!(r.encode(), s);
    }
}
