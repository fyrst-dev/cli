//! Opt-in sales-channel URL rewrite after restore.
//!
//! Overlay bash keeps “requested?” and live-refuse. The rewrite itself is
//! `bin/console fyrst:sales-channel:rewrite-urls` via compose `web` — not SQL.

use super::env::{existing_compose_files, ShopEnv};
use super::error::Error;
use super::live::LiveSignals;
use super::mysql::{compose_argv, compose_cli_log};
use std::path::Path;

pub fn rewrite_requested(env: &ShopEnv) -> bool {
    env.get("SYNC_REWRITE_APP_URL").is_some() || env.get("SYNC_REWRITE_URL_MAP").is_some()
}

/// Hard refuse rewrite on a live consumer. `SYNC_ALLOW_LIVE_RESTORE=1` does not bypass this.
pub fn assert_not_live_rewrite(signals: &LiveSignals) -> Result<(), Error> {
    if !signals.is_live() {
        return Ok(());
    }
    Err(Error::fail(format!(
        "Refusing sales-channel domain rewrite on a live host (SYNC_ENV={}, SHOPWARE_DEPLOY_ENV={}). Unset SYNC_REWRITE_APP_URL / SYNC_REWRITE_URL_MAP. Rewrite is never allowed on live (SYNC_ALLOW_LIVE_RESTORE=1 does not bypass this).",
        signals.sync_env.as_deref().unwrap_or("unset"),
        signals.deploy_env.as_deref().unwrap_or("unset"),
    )))
}

pub fn console_flag_args(env: &ShopEnv, checkout: &str, dry_run: bool) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(url) = env.get("SYNC_REWRITE_APP_URL") {
        args.push(format!("--app-url={url}"));
    }
    if let Some(map) = env.get("SYNC_REWRITE_URL_MAP") {
        args.push(format!("--map={map}"));
    }
    args.push(format!(
        "--deploy-env={}",
        env.get("SHOPWARE_DEPLOY_ENV").unwrap_or("")
    ));
    args.push(format!("--sync-env={}", env.get("SYNC_ENV").unwrap_or("")));
    args.push(format!("--checkout-basename={checkout}"));
    if dry_run {
        args.push("--dry-run".into());
    }
    args
}

pub fn compose_rewrite_args(env: &ShopEnv, compose_dir: &Path, dry_run: bool) -> Vec<String> {
    let files = existing_compose_files(compose_dir);
    let checkout = super::live::shop_basename(compose_dir);
    let mut args = compose_argv(&files);
    args.extend([
        "run".into(),
        "--rm".into(),
        "--pull".into(),
        "never".into(),
        "--entrypoint".into(),
        "php".into(),
        "web".into(),
        "bin/console".into(),
        "fyrst:sales-channel:rewrite-urls".into(),
    ]);
    args.extend(console_flag_args(env, &checkout, dry_run));
    args
}

pub fn rewrite_log_line(env: &ShopEnv, compose_dir: &Path, dry_run: bool) -> String {
    let files = existing_compose_files(compose_dir);
    let checkout = super::live::shop_basename(compose_dir);
    let flags = console_flag_args(env, &checkout, dry_run);
    format!(
        "{} run --rm --pull never --entrypoint php web bin/console fyrst:sales-channel:rewrite-urls {}",
        compose_cli_log(&files),
        flags.join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::super::live::LiveSignals;
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn env_from(pairs: &[(&str, &str)]) -> ShopEnv {
        let mut vars = HashMap::new();
        for (k, v) in pairs {
            vars.insert((*k).to_string(), (*v).to_string());
        }
        ShopEnv::from_vars(PathBuf::from("/shops/acme-staging"), vars)
    }

    #[test]
    fn requested_only_when_url_or_map_set() {
        assert!(!rewrite_requested(&env_from(&[])));
        assert!(rewrite_requested(&env_from(&[(
            "SYNC_REWRITE_APP_URL",
            "https://staging.example.com"
        )])));
        assert!(rewrite_requested(&env_from(&[(
            "SYNC_REWRITE_URL_MAP",
            "https://shop.example.com=https://staging.example.com"
        )])));
    }

    #[test]
    fn console_args_omit_dry_run_when_off() {
        let env = env_from(&[
            ("SYNC_REWRITE_APP_URL", "https://staging.example.com"),
            ("SHOPWARE_DEPLOY_ENV", "staging"),
            ("SYNC_ENV", "staging"),
        ]);
        let args = console_flag_args(&env, "acme-staging", false);
        let joined = args.join(" ");
        assert!(!args.iter().any(|a| a == "--dry-run"), "{joined}");
        assert!(
            joined.contains("--app-url=https://staging.example.com"),
            "{joined}"
        );
        assert!(joined.contains("--deploy-env=staging"), "{joined}");
        assert!(joined.contains("--sync-env=staging"), "{joined}");
        assert!(
            joined.contains("--checkout-basename=acme-staging"),
            "{joined}"
        );
        assert!(!joined.contains("--map="), "{joined}");
    }

    #[test]
    fn console_args_add_dry_run_and_map() {
        let env = env_from(&[(
            "SYNC_REWRITE_URL_MAP",
            "https://shop.example.com=https://staging.example.com",
        )]);
        let args = console_flag_args(&env, "acme-staging", true);
        let joined = args.join(" ");
        assert!(args.iter().any(|a| a == "--dry-run"), "{joined}");
        assert!(
            joined.contains("--map=https://shop.example.com=https://staging.example.com"),
            "{joined}"
        );
        assert!(!joined.contains("--app-url="), "{joined}");
    }

    #[test]
    fn compose_line_is_console_not_sql() {
        let env = env_from(&[("SYNC_REWRITE_APP_URL", "https://staging.example.com")]);
        let line = rewrite_log_line(&env, Path::new("/shops/acme-staging"), true);
        assert!(line.contains("fyrst:sales-channel:rewrite-urls"), "{line}");
        assert!(
            line.contains("run --rm --pull never --entrypoint php"),
            "{line}"
        );
        assert!(line.contains("--dry-run"), "{line}");
        assert!(
            !line.to_ascii_lowercase().contains("update sales_channel"),
            "{line}"
        );
        let args = compose_rewrite_args(&env, Path::new("/shops/acme-staging"), false);
        assert!(args.contains(&"fyrst:sales-channel:rewrite-urls".to_string()));
        assert!(!args.iter().any(|a| a == "--dry-run"));
    }

    #[test]
    fn live_rewrite_refused_even_when_restore_allowed() {
        let signals = LiveSignals {
            sync_env: Some("live".into()),
            deploy_env: Some("live".into()),
            shop_basename: "acme-live".into(),
            hostname_short: Some("vps-1".into()),
            allow_env: true,
        };
        let err = assert_not_live_rewrite(&signals).unwrap_err();
        assert!(
            err.to_string()
                .contains("Refusing sales-channel domain rewrite on a live host"),
            "{err}"
        );
        assert!(
            err.to_string()
                .contains("SYNC_ALLOW_LIVE_RESTORE=1 does not bypass"),
            "{err}"
        );
        let staging = LiveSignals {
            sync_env: Some("staging".into()),
            deploy_env: Some("staging".into()),
            shop_basename: "acme-staging".into(),
            hostname_short: Some("vps-1".into()),
            allow_env: false,
        };
        assert!(assert_not_live_rewrite(&staging).is_ok());
    }
}
