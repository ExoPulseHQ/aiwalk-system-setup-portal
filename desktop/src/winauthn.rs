//! Windows: security keys only through Windows' own WebAuthn service (webauthn.dll); an ordinary program may list them
//! but not talk to them. The two operations sealed notes need, with the same relying party, credential and salt as
//! the CTAP path (fidokey.rs), so a key set up on Linux opens on Windows and back: a credential with hmac-secret, and
//! the hmac-secret of credentials for salts. Windows shows its own window for the PIN and the touch.
// ponytail: needs Windows 11 22H2 or later (WebAuthn API 4+, raw hmac-secret salts); older Windows says so at the call

use crate::fidokey::Miss;
use windows_sys::Win32::Networking::WindowsWebServices::*;

const RP: &str = crate::fidokey::RP_ID;
const NOT_FOUND: i32 = 0x8009_0011u32 as i32;       // NTE_NOT_FOUND: none of the credentials is on this key
const CANCELLED: i32 = 0x8009_0036u32 as i32;       // NTE_USER_CANCELLED

fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain([0]).collect() }

fn window() -> windows_sys::Win32::Foundation::HWND { unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() } }

fn api_ok() -> Result<(), String> {
    if unsafe { WebAuthNGetApiVersionNumber() } < WEBAUTHN_API_VERSION_4 {
        return Err("Sealed notes need Windows 11 22H2 or later on this computer.".into())
    }
    Ok(())
}

fn client_data(kind: &str) -> (Vec<u8>, Vec<u16>) {
    (format!(r#"{{"type":"{kind}","challenge":"c2VhbGVk","origin":"{RP}"}}"#).into_bytes(), wide("SHA-256"))
}

/// makeCredential through Windows: not discoverable, hmac-secret on, user verification required (Windows asks the PIN).
/// Its credential ID.
pub fn make_credential() -> Result<Vec<u8>, String> {
    api_ok()?;
    let (rp_id, rp_name, user_name, kind) = (wide(RP), wide(RP), wide("age"), wide("public-key"));
    let mut user_id = [0u8; 16];
    let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut user_id);
    let rp = WEBAUTHN_RP_ENTITY_INFORMATION { dwVersion: WEBAUTHN_RP_ENTITY_INFORMATION_CURRENT_VERSION, pwszId: rp_id.as_ptr(), pwszName: rp_name.as_ptr(), pwszIcon: std::ptr::null() };
    let user = WEBAUTHN_USER_ENTITY_INFORMATION { dwVersion: WEBAUTHN_USER_ENTITY_INFORMATION_CURRENT_VERSION, cbId: 16, pbId: user_id.as_mut_ptr(), pwszName: user_name.as_ptr(), pwszIcon: std::ptr::null(), pwszDisplayName: user_name.as_ptr() };
    let mut param = WEBAUTHN_COSE_CREDENTIAL_PARAMETER { dwVersion: WEBAUTHN_COSE_CREDENTIAL_PARAMETER_CURRENT_VERSION, pwszCredentialType: kind.as_ptr(), lAlg: WEBAUTHN_COSE_ALGORITHM_ECDSA_P256_WITH_SHA256 };
    let params = WEBAUTHN_COSE_CREDENTIAL_PARAMETERS { cCredentialParameters: 1, pCredentialParameters: &mut param };
    let (mut json, hash) = client_data("webauthn.create");
    let cd = WEBAUTHN_CLIENT_DATA { dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION, cbClientDataJSON: json.len() as u32, pbClientDataJSON: json.as_mut_ptr(), pwszHashAlgId: hash.as_ptr() };
    let (ext_id, mut on) = (wide("hmac-secret"), 1i32);
    let mut ext = WEBAUTHN_EXTENSION { pwszExtensionIdentifier: ext_id.as_ptr(), cbExtension: 4, pvExtension: (&mut on as *mut i32).cast() };
    let mut opts = WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS::default();
    opts.dwVersion = WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS_VERSION_3;
    opts.dwTimeoutMilliseconds = 60_000;
    opts.Extensions = WEBAUTHN_EXTENSIONS { cExtensions: 1, pExtensions: &mut ext };
    opts.dwAuthenticatorAttachment = WEBAUTHN_AUTHENTICATOR_ATTACHMENT_CROSS_PLATFORM;
    opts.dwUserVerificationRequirement = WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED;
    opts.dwAttestationConveyancePreference = WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_NONE;
    let mut att: *mut WEBAUTHN_CREDENTIAL_ATTESTATION = std::ptr::null_mut();
    // SAFETY: every pointer handed over points into locals that outlive the call; the result is freed below
    let hr = unsafe { WebAuthNAuthenticatorMakeCredential(window(), &rp, &user, &params, &cd, &opts, &mut att) };
    if hr == CANCELLED { return Err("The key was not touched, or the Windows window was closed.".into()) }
    if hr != 0 || att.is_null() { return Err(format!("Windows could not make the credential ({hr:#010x}).")) }
    // SAFETY: Windows filled `att`; the credential ID is copied out before it is freed
    let cred = unsafe { std::slice::from_raw_parts((*att).pbCredentialId, (*att).cbCredentialId as usize).to_vec() };
    unsafe { WebAuthNFreeCredentialAttestation(att) };
    Ok(cred)
}

/// The hmac-secret of one credential for a salt. With `uv`, Windows asks the PIN (the "with UV" secret, as the
/// stanzas' PIN flag says).
pub fn hmac_secret(uv: bool, cred: &[u8], salt: &[u8; 32]) -> Result<[u8; 32], Miss> {
    hmac_secret_any(uv, &[(cred.to_vec(), *salt)]).map(|(_, s)| s)
}

/// The hmac-secret of whichever of these credentials is on the plugged-in key, each with its own salt, in one Windows
/// prompt (asking one credential at a time would show a prompt for every other holder's stanza): which one, and its
/// secret.
pub fn hmac_secret_any(uv: bool, creds: &[(Vec<u8>, [u8; 32])]) -> Result<(usize, [u8; 32]), Miss> {
    api_ok().map_err(Miss::Failed)?;
    let rp = wide(RP);
    let kind = wide("public-key");
    let mut ids: Vec<Vec<u8>> = creds.iter().map(|(c, _)| c.clone()).collect();
    let mut salts: Vec<[u8; 32]> = creds.iter().map(|(_, s)| *s).collect();
    let mut list: Vec<WEBAUTHN_CREDENTIAL> = ids.iter_mut().map(|id| WEBAUTHN_CREDENTIAL { dwVersion: WEBAUTHN_CREDENTIAL_CURRENT_VERSION, cbId: id.len() as u32, pbId: id.as_mut_ptr(), pwszCredentialType: kind.as_ptr() }).collect();
    let mut salt_structs: Vec<WEBAUTHN_HMAC_SECRET_SALT> = salts.iter_mut().map(|s| WEBAUTHN_HMAC_SECRET_SALT { cbFirst: 32, pbFirst: s.as_mut_ptr(), cbSecond: 0, pbSecond: std::ptr::null_mut() }).collect();
    let mut per_cred: Vec<WEBAUTHN_CRED_WITH_HMAC_SECRET_SALT> = ids.iter_mut().zip(salt_structs.iter_mut())
        .map(|(id, s)| WEBAUTHN_CRED_WITH_HMAC_SECRET_SALT { cbCredID: id.len() as u32, pbCredID: id.as_mut_ptr(), pHmacSecretSalt: s }).collect();
    let mut values = WEBAUTHN_HMAC_SECRET_SALT_VALUES { pGlobalHmacSalt: std::ptr::null_mut(), cCredWithHmacSecretSaltList: per_cred.len() as u32, pCredWithHmacSecretSaltList: per_cred.as_mut_ptr() };
    let (mut json, hash) = client_data("webauthn.get");
    let cd = WEBAUTHN_CLIENT_DATA { dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION, cbClientDataJSON: json.len() as u32, pbClientDataJSON: json.as_mut_ptr(), pwszHashAlgId: hash.as_ptr() };
    let mut opts = WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS::default();
    opts.dwVersion = WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS_VERSION_5;
    opts.dwTimeoutMilliseconds = 60_000;
    opts.CredentialList = WEBAUTHN_CREDENTIALS { cCredentials: list.len() as u32, pCredentials: list.as_mut_ptr() };
    opts.dwAuthenticatorAttachment = WEBAUTHN_AUTHENTICATOR_ATTACHMENT_CROSS_PLATFORM;
    opts.dwUserVerificationRequirement = if uv { WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED } else { WEBAUTHN_USER_VERIFICATION_REQUIREMENT_DISCOURAGED };
    // raw salts, as CTAP's hmac-secret takes them (not WebAuthn PRF inputs, which Windows would hash first)
    opts.dwFlags = WEBAUTHN_AUTHENTICATOR_HMAC_SECRET_VALUES_FLAG;
    opts.pHmacSecretSaltValues = &mut values;
    let mut a: *mut WEBAUTHN_ASSERTION = std::ptr::null_mut();
    // SAFETY: every pointer handed over points into locals that outlive the call; the result is freed below
    let hr = unsafe { WebAuthNAuthenticatorGetAssertion(window(), rp.as_ptr(), &cd, &opts, &mut a) };
    if hr == NOT_FOUND { return Err(Miss::NotThisKey) }
    if hr == CANCELLED { return Err(Miss::Failed("The key was not touched, or the Windows window was closed.".into())) }
    if hr != 0 || a.is_null() { return Err(Miss::Failed(format!("Windows could not ask the key ({hr:#010x})."))) }
    // SAFETY: Windows filled `a`; what is needed is copied out before it is freed
    let found = unsafe {
        let used = std::slice::from_raw_parts((*a).Credential.pbId, (*a).Credential.cbId as usize);
        let which = creds.iter().position(|(c, _)| c.as_slice() == used);
        let h = (*a).pHmacSecret;
        let secret = (!h.is_null() && (*h).cbFirst == 32).then(|| std::slice::from_raw_parts((*h).pbFirst, 32).try_into().unwrap());
        WebAuthNFreeAssertion(a);
        which.zip(secret)
    };
    found.ok_or_else(|| Miss::Failed("Windows gave no hmac-secret for the key.".into()))
}
