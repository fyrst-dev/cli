//! Live-host refuse guards (recipes `lib/sync-live.sh`).
//!
//! `shopware sync apply` matches the overlay: refuse when the consumer looks
//! live unless `SYNC_ALLOW_LIVE_RESTORE=1`.
//!
//! Standalone `shopware db import` uses the same live detection but allows an
//! explicit `--allow-live` (or the same env flag) so staging imports stay easy
//! while live is never a silent default.

use super::env::ShopEnv;
use super::error::Error;
use std::path::Path;
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LivePolicy {
    /// Standalone import: refuse live unless `--allow-live` or env `1`.
    DbImport { allow_live_flag: bool },
    /// `sync apply` / `sync pull`: refuse live unless `SYNC_ALLOW_LIVE_RESTORE=1`.
    SyncRestore,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveSignals {
    pub sync_env: Option<String>,
    pub deploy_env: Option<String>,
    pub shop_basename: String,
    pub hostname_short: Option<String>,
    pub allow_env: bool,
}

impl LiveSignals {
    pub fn from_shop(env: &ShopEnv) -> Self {
        Self {
            sync_env: env.get("SYNC_ENV").map(str::to_string),
            deploy_env: env.get("SHOPWARE_DEPLOY_ENV").map(str::to_string),
            shop_basename: shop_basename(&env.compose_dir),
            hostname_short: hostname_short(),
            allow_env: env.get("SYNC_ALLOW_LIVE_RESTORE") == Some("1"),
        }
    }

    pub fn is_live(&self) -> bool {
        is_live_token(self.sync_env.as_deref())
            || is_live_token(self.deploy_env.as_deref())
            || is_live_token(Some(&self.shop_basename))
            || is_live_token(self.hostname_short.as_deref())
    }

    pub fn allowed(&self, policy: LivePolicy) -> bool {
        match policy {
            LivePolicy::DbImport { allow_live_flag } => allow_live_flag || self.allow_env,
            LivePolicy::SyncRestore => self.allow_env,
        }
    }
}

fn is_live_token(s: Option<&str>) -> bool {
    s.map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.eq_ignore_ascii_case("live"))
        .unwrap_or(false)
}

fn hostname_short() -> Option<String> {
    for args in [&["-s"][..], &[][..]] {
        let out = match Command::new("hostname").args(args).output() {
            Ok(o) => o,
            Err(_) => continue,
        };
        if !out.status.success() {
            continue;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            return Some(s);
        }
    }
    None
}

/// Returns a warning to print when live import is explicitly allowed.
pub fn assert_not_live(signals: &LiveSignals, policy: LivePolicy) -> Result<Option<String>, Error> {
    if !signals.is_live() {
        return Ok(None);
    }
    if signals.allowed(policy) {
        let via = match policy {
            LivePolicy::DbImport { allow_live_flag } if allow_live_flag => "--allow-live",
            _ => "SYNC_ALLOW_LIVE_RESTORE=1",
        };
        return Ok(Some(format!(
            "WARNING: {via} — importing onto a live host (disaster recovery). This is not the live→staging sync path."
        )));
    }
    let sync = signals.sync_env.as_deref().unwrap_or("unset");
    let deploy = signals.deploy_env.as_deref().unwrap_or("unset");
    let host = signals.hostname_short.as_deref().unwrap_or("unset");
    match policy {
        LivePolicy::SyncRestore => Err(Error::fail(format!(
            "Refusing restore/sync on a live host (SYNC_ENV={sync}, SHOPWARE_DEPLOY_ENV={deploy}, checkout={}, hostname={host}). Runtime sync is pull-only onto staging/playground/dev. Live backups use deploy/backup-runtime.sh; live restore is BACKUP_ALLOW_LIVE_RESTORE=1 (quarterly DR drill). For this CLI, set SYNC_ALLOW_LIVE_RESTORE=1 to override.",
            signals.shop_basename
        ))),
        LivePolicy::DbImport { .. } => Err(Error::fail(format!(
            "Refusing database import on a live host (SYNC_ENV={sync}, SHOPWARE_DEPLOY_ENV={deploy}, checkout={}, hostname={host}). Pass --allow-live or set SYNC_ALLOW_LIVE_RESTORE=1. Casual imports are for staging/playground/dev; live is opt-in only.",
            signals.shop_basename
        ))),
    }
}

pub fn shop_basename(compose_dir: &Path) -> String {
    compose_dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signals(deploy: &str, basename: &str, host: Option<&str>, allow: bool) -> LiveSignals {
        LiveSignals {
            sync_env: None,
            deploy_env: Some(deploy.into()),
            shop_basename: basename.into(),
            hostname_short: host.map(str::to_string),
            allow_env: allow,
        }
    }

    #[test]
    fn staging_is_not_live() {
        let s = signals("staging", "shop", Some("laptop"), false);
        assert!(!s.is_live());
        assert!(assert_not_live(
            &s,
            LivePolicy::DbImport {
                allow_live_flag: false
            }
        )
        .unwrap()
        .is_none());
        assert!(assert_not_live(&s, LivePolicy::SyncRestore)
            .unwrap()
            .is_none());
    }

    #[test]
    fn live_deploy_env_refuses_import_without_flag() {
        let s = signals("live", "shop", Some("laptop"), false);
        assert!(s.is_live());
        let err = assert_not_live(
            &s,
            LivePolicy::DbImport {
                allow_live_flag: false,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("--allow-live"), "{err}");
    }

    #[test]
    fn live_import_with_flag_warns() {
        let s = signals("live", "shop", Some("laptop"), false);
        let warn = assert_not_live(
            &s,
            LivePolicy::DbImport {
                allow_live_flag: true,
            },
        )
        .unwrap()
        .unwrap();
        assert!(warn.starts_with("WARNING:"), "{warn}");
        assert!(warn.contains("--allow-live"), "{warn}");
    }

    #[test]
    fn restore_needs_env_even_if_import_flag() {
        let s = signals("live", "shop", Some("laptop"), false);
        let err = assert_not_live(&s, LivePolicy::SyncRestore).unwrap_err();
        assert!(err.to_string().contains("SYNC_ALLOW_LIVE_RESTORE"), "{err}");
        let s = signals("live", "shop", Some("laptop"), true);
        let warn = assert_not_live(&s, LivePolicy::SyncRestore)
            .unwrap()
            .unwrap();
        assert!(warn.contains("SYNC_ALLOW_LIVE_RESTORE"), "{warn}");
    }

    #[test]
    fn checkout_named_live() {
        let s = LiveSignals {
            sync_env: None,
            deploy_env: Some("staging".into()),
            shop_basename: "live".into(),
            hostname_short: Some("box".into()),
            allow_env: false,
        };
        assert!(s.is_live());
    }

    #[test]
    fn basename_helper() {
        assert_eq!(shop_basename(Path::new("/shops/acme")), "acme");
    }
}
