//! The team this build serves: its GitHub OAuth App, the repositories it reads and the Cloudflare Access
//! organisation that guards its machines. Read once, on first use.
//!
//! The values ship in `desktop/aiwalk.config.json`, so another team forks the repository, edits that file and builds
//! its own app. A computer may also lay a file over them key by key, without a rebuild: the one `AIWALK_CONFIG`
//! names, else `~/.config/aiwalk-setup/config.json`. A missing file changes nothing; a file that is not a JSON
//! object, or a value of the wrong type, is ignored with one line on stderr, so a typo never stops the app.

use serde::Deserialize;
use serde_json::Value;
use std::sync::OnceLock;

const SHIPPED: &str = include_str!("../aiwalk.config.json");

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Cloudflare {
    /// Every machine is ssh-<name> and status-<name> under this zone, leading dot included.
    pub zone: String,
    /// The Access application with no machine behind it, open to anyone who signs in with GitHub.
    pub enroll: String,
    /// The Access organisation: tokens from any other are refused.
    pub team: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Vault {
    pub name: String,
    pub repo: String,
    pub about: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Config {
    /// The OAuth App with the device flow on. A client id is not a secret.
    pub github_client_id: String,
    /// owner/name whose Releases hold this app's installers.
    pub update_repo: String,
    /// The repository, in the first vault's organisation, where access requests travel as issues.
    pub requests_repo: String,
    pub cloudflare: Cloudflare,
    /// Repositories of the organisation an intern is invited to read; the requests repository is added if missing.
    pub intern_repos: Vec<String>,
    /// The first one is the team's own: its owner is the organisation the app works for.
    pub vaults: Vec<Vault>,
}

impl Config {
    /// The GitHub organisation: the owner of the first vault.
    pub fn org(&self) -> &str { self.vaults.first().and_then(|v| v.repo.split('/').next()).unwrap_or_default() }
    /// owner/name of the requests repository.
    pub fn requests(&self) -> String { format!("{}/{}", self.org(), self.requests_repo) }
}

/// `over` laid on `base`, key by key; nested objects (cloudflare) are merged the same way, lists replaced whole.
fn merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => for (k, v) in o {
            match b.get_mut(&k) { Some(slot @ Value::Object(_)) if v.is_object() => merge(slot, v), _ => { b.insert(k, v); } }
        },
        (b, o) => *b = o,
    }
}

/// The shipped values with `over` (a file's text) laid on them. Err names why the file was not used.
pub fn build(over: Option<&str>) -> (Config, Option<String>) {
    let mut base: Value = serde_json::from_str(SHIPPED).expect("desktop/aiwalk.config.json is valid JSON");
    let shipped: Config = serde_json::from_value(base.clone()).expect("desktop/aiwalk.config.json has every field");
    let Some(text) = over else { return (shipped, None) };
    let over = match serde_json::from_str::<Value>(text) {
        Ok(v @ Value::Object(_)) => v,
        Ok(_) => return (shipped, Some("is not a JSON object".into())),
        Err(e) => return (shipped, Some(format!("is not JSON: {e}"))),
    };
    merge(&mut base, over);
    match serde_json::from_value::<Config>(base) {
        Ok(c) if c.vaults.is_empty() => (shipped, Some("lists no vaults".into())),
        Ok(c) => (c, None),
        Err(e) => (shipped, Some(format!("has a value of the wrong type: {e}"))),
    }
}

/// The file this computer lays over the shipped values, if any.
fn local_file() -> std::path::PathBuf {
    match std::env::var_os("AIWALK_CONFIG").filter(|v| !v.is_empty()) {
        Some(p) => p.into(),
        None => crate::home().join(".config").join("aiwalk-setup").join("config.json"),
    }
}

pub fn get() -> &'static Config {
    static CONFIG: OnceLock<Config> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let file = local_file();
        let text = std::fs::read_to_string(&file).ok();
        let (c, err) = build(text.as_deref());
        if let Some(e) = err { eprintln!("aiwalk-setup: {} {e}; using the built-in settings", file.display()) }
        c
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_values_are_the_ones_the_app_had() {
        let (c, err) = build(None);
        assert!(err.is_none());
        assert_eq!(c.github_client_id, "Ov23libbDrZ77FJNDo4L");
        assert_eq!(c.update_repo, "ExoPulseHQ/aiwalk-system-setup-portal");
        assert_eq!(c.requests(), "ExoPulseHQ/exo-access-requests");
        assert_eq!(c.org(), "ExoPulseHQ");
        assert_eq!(c.cloudflare.zone, ".aiwalkcorp.com");
        assert_eq!(c.cloudflare.enroll, "enroll.aiwalkcorp.com");
        assert_eq!(c.cloudflare.team, "aiwalkcorp.cloudflareaccess.com");
        assert_eq!(c.vaults[0].repo, "ExoPulseHQ/exo-book");
        assert_eq!(c.intern_repos, ["aiwalk-system-setup-portal", "exo-access-requests", "exo-book", "exo-papers"]);
    }

    #[test]
    fn a_file_changes_only_the_keys_it_names() {
        let (c, err) = build(Some(r#"{"cloudflare": {"zone": ".lumen.example"}, "vaults": [{"name": "Lumen", "repo": "LumenLamp/book", "about": "Docs"}]}"#));
        assert!(err.is_none());
        assert_eq!(c.cloudflare.zone, ".lumen.example");
        assert_eq!(c.cloudflare.team, "aiwalkcorp.cloudflareaccess.com");
        assert_eq!(c.org(), "LumenLamp");
        assert_eq!(c.requests(), "LumenLamp/exo-access-requests");
        assert_eq!(c.vaults.len(), 1);
    }

    #[test]
    fn a_broken_file_keeps_the_shipped_values() {
        let shipped = build(None).0;
        for bad in ["not json", "[1, 2]", r#"{"vaults": []}"#, r#"{"cloudflare": {"zone": 3}}"#, r#"{"vaults": "x"}"#] {
            let (c, err) = build(Some(bad));
            assert!(err.is_some(), "{bad} should be refused");
            assert_eq!(c, shipped);
        }
    }
}
