//! Small secrets in the system's own keyring, never in a file: Secret Service on Linux, Keychain on macOS,
//! Credential Manager on Windows. `key` names one secret ("github").

/// Runs `f` on a thread of its own; None if it panicked. The blocking Secret Service client starts a runtime, which
/// panics on a thread that already drives one, and every command the page calls runs on such a thread: the page
/// then waits for an answer that never comes. Every keyring call in the app goes through here.
pub(crate) fn apart<T: Send>(f: impl FnOnce() -> T + Send) -> Option<T> { std::thread::scope(|s| s.spawn(f).join().ok()) }

#[cfg(target_os = "linux")]
mod imp {
    use secret_service::{blocking::SecretService, EncryptionType};
    use std::collections::HashMap;
    use super::apart;
    pub fn load(key: &str) -> Option<String> { apart(|| load_here(key)).flatten() }
    pub fn save(key: &str, secret: &str) -> Result<(), String> { apart(|| save_here(key, secret)).unwrap_or_else(|| Err("the keyring did not answer".into())) }
    pub fn forget(key: &str) { apart(|| forget_here(key)); }
    fn attrs(key: &str) -> HashMap<&str, &str> { HashMap::from([("xdg:schema", "com.aiwalk.setup.Secret"), ("key", key)]) }
    fn load_here(key: &str) -> Option<String> {
        let ss = SecretService::connect(EncryptionType::Dh).ok()?;
        let found = ss.search_items(attrs(key)).ok()?;
        let item = found.unlocked.into_iter().next().or_else(|| { let i = found.locked.into_iter().next()?; i.unlock().ok()?; Some(i) })?;
        String::from_utf8(item.get_secret().ok()?).ok()
    }
    fn save_here(key: &str, secret: &str) -> Result<(), String> {
        let ss = SecretService::connect(EncryptionType::Dh).map_err(|e| e.to_string())?;
        let c = ss.get_default_collection().map_err(|e| e.to_string())?;
        if c.is_locked().unwrap_or(false) { c.unlock().map_err(|e| e.to_string())? }
        c.create_item(&format!("aIwalk System Setup: {key}"), attrs(key), secret.as_bytes(), true, "text/plain").map(|_| ()).map_err(|e| e.to_string())
    }
    fn forget_here(key: &str) {
        let Ok(ss) = SecretService::connect(EncryptionType::Dh) else { return };
        if let Ok(found) = ss.search_items(attrs(key)) { for i in found.unlocked.into_iter().chain(found.locked) { let _ = i.delete(); } };
    }
}

#[cfg(any(target_os = "macos", windows))]
mod imp {
    fn entry(key: &str) -> Result<keyring::Entry, String> { keyring::Entry::new("aIwalk System Setup", key).map_err(|e| e.to_string()) }
    pub fn load(key: &str) -> Option<String> { entry(key).ok()?.get_password().ok() }
    pub fn save(key: &str, secret: &str) -> Result<(), String> { entry(key)?.set_password(secret).map_err(|e| e.to_string()) }
    pub fn forget(key: &str) { if let Ok(e) = entry(key) { let _ = e.delete_credential(); } }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod imp {
    pub fn load(_: &str) -> Option<String> { None }
    pub fn save(_: &str, _: &str) -> Result<(), String> { Err("this system has no keyring this app knows".into()) }
    pub fn forget(_: &str) {}
}

pub use imp::{forget, load, save};
