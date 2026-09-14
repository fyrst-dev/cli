//! Sales-channel URL rewrite after restore.
//!
//! Target URL is `APP_URL`. On live, rewrite is skipped (APP_URL is a shop
//! runtime var, not a restore opt-in — refusing it would block live DR).

use super::env::{existing_compose_files, ShopEnv};
use super::error::Error;
use super::live::LiveSignals;
use super::mysql::{compose_argv, compose_cli_log, require_docker};
use std::path::Path;
use std::process::Command;

pub fn rewrite_app_url(env: &ShopEnv) -> Option<&str> {
    env.get("APP_URL")
}

pub fn rewrite_requested(env: &ShopEnv) -> bool {
    rewrite_app_url(env).is_some()
}

/// Rewrite never runs on live. Returns Ok so restore/DR can continue.
pub fn assert_not_live_rewrite(signals: &LiveSignals) -> Result<(), Error> {
    if !signals.is_live() {
        return Ok(());
    }
    Ok(())
}

pub fn should_rewrite(env: &ShopEnv, signals: &LiveSignals) -> bool {
    rewrite_requested(env) && !signals.is_live()
}

pub fn console_flag_args(env: &ShopEnv, checkout: &str, dry_run: bool) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(url) = rewrite_app_url(env) {
        args.push(format!("--app-url={url}"));
    }
    args.push(format!(
        "--deploy-env={}",
        env.get("SHOPWARE_DEPLOY_ENV").unwrap_or("")
    ));
    args.push(format!(
        "--sync-env={}",
        env.get("SHOPWARE_DEPLOY_ENV").unwrap_or("")
    ));
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

/// Names used by `shopware sync pull` (same predicates as restore).
pub fn requested(env: &ShopEnv) -> bool {
    rewrite_requested(env)
}

pub fn assert_not_live(signals: &LiveSignals) -> Result<(), Error> {
    assert_not_live_rewrite(signals)
}

pub fn skip_without_db_log() -> &'static str {
    "APP_URL set but db was skipped — not rewriting sales_channel_domain"
}

pub fn skip_live_log() -> &'static str {
    "Skipping sales-channel domain rewrite on a live host (rewrite is never applied on live)"
}

pub fn maybe_rewrite(
    env: &ShopEnv,
    signals: &LiveSignals,
    compose_dir: &Path,
    files: &[String],
    want_db_restored: bool,
    dry_run: bool,
) -> Result<(), Error> {
    if !rewrite_requested(env) {
        return Ok(());
    }
    if signals.is_live() {
        println!("==> {}", skip_live_log());
        return Ok(());
    }
    if !want_db_restored {
        println!("==> {}", skip_without_db_log());
        return Ok(());
    }
    println!(
        "==> Sales_channel_domain rewrite via fyrst:sales-channel:rewrite-urls from APP_URL (sales channel domains only; media CDN / plugin configs / payment webhooks are not updated)"
    );
    if files.is_empty() {
        return Err(Error::fail(
            "No compose files found under shop root; cannot run fyrst:sales-channel:rewrite-urls.",
        ));
    }
    let checkout = super::live::shop_basename(compose_dir);
    let args = {
        let mut a = compose_argv(files);
        a.extend([
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
        a.extend(console_flag_args(env, &checkout, dry_run));
        a
    };
    if dry_run {
        println!("==> DRY-RUN docker {}", args.join(" "));
        return Ok(());
    }
    require_docker()?;
    let status = Command::new("docker")
        .args(&args)
        .current_dir(compose_dir)
        .status()
        .map_err(|e| Error::fail(format!("could not exec docker compose rewrite: {e}")))?;
    if !status.success() {
        return Err(Error::fail(
            "fyrst:sales-channel:rewrite-urls failed. composer update fyrst/shopware-cd so the command and FyrstShopwareCdBundle exist, then composer recipes:update fyrst/shopware-cd.",
        ));
    }
    Ok(())
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
    fn requested_only_when_app_url_set() {
        assert!(!rewrite_requested(&env_from(&[])));
        assert!(rewrite_requested(&env_from(&[(
            "APP_URL",
            "https://staging.example.com"
        )])));
    }

    #[test]
    fn console_args_omit_dry_run_when_off() {
        let env = env_from(&[
            ("APP_URL", "https://staging.example.com"),
            ("SHOPWARE_DEPLOY_ENV", "staging"),
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
    fn console_args_add_dry_run() {
        let env = env_from(&[("APP_URL", "https://staging.example.com")]);
        let args = console_flag_args(&env, "acme-staging", true);
        let joined = args.join(" ");
        assert!(args.iter().any(|a| a == "--dry-run"), "{joined}");
        assert!(
            joined.contains("--app-url=https://staging.example.com"),
            "{joined}"
        );
    }

    #[test]
    fn compose_line_is_console_not_sql() {
        let env = env_from(&[("APP_URL", "https://staging.example.com")]);
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
    fn live_skips_rewrite_so_dr_can_continue() {
        let signals = LiveSignals {
            deploy_env: Some("live".into()),
            shop_basename: "acme-live".into(),
            hostname_short: Some("vps-1".into()),
            allow_env: true,
        };
        assert!(assert_not_live_rewrite(&signals).is_ok());
        let env = env_from(&[("APP_URL", "https://shop.example.com")]);
        assert!(!should_rewrite(&env, &signals));
        let staging = LiveSignals {
            deploy_env: Some("staging".into()),
            shop_basename: "acme-staging".into(),
            hostname_short: Some("vps-1".into()),
            allow_env: false,
        };
        assert!(should_rewrite(&env, &staging));
    }
}
