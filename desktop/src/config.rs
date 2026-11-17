//! The team this build serves: its GitHub OAuth App, the repositories it reads and the Cloudflare Access
//! organisation that guards its machines. Read once, on first use.
//!
//! The values ship in `desktop/aiwalk.config.json` and are compiled in, so another team forks the repository, edits
//! that file and builds its own app. Nothing on the computer changes them: where updates are downloaded from and
//! which Access organisation is trusted are not left to a file in the home folder.

use serde::Deserialize;
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

pub fn get() -> &'static Config {
    static CONFIG: OnceLock<Config> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let c: Config = serde_json::from_str(SHIPPED).expect("desktop/aiwalk.config.json is valid and has every field");
        // ponytail: checked on first use, not at build; a build.rs check if a broken file ever ships
        assert!(c.vaults.first().is_some_and(|v| v.repo.contains('/')), "aiwalk.config.json: the first vault is owner/name");
        assert!(c.cloudflare.zone.starts_with('.'), "aiwalk.config.json: cloudflare.zone starts with a dot");
        c
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_values_are_the_ones_the_app_had() {
        let c = get();
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
}
