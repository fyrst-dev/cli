//! Sales-channel domain update after a non-live DB import.
//!
//! Shopware `sales-channel:update:domain` takes the new host. The host is
//! parsed from `APP_URL` (scheme and path stripped; port is not passed).
//! On live, the command is skipped (`APP_URL` is a shop runtime var, not a
//! restore opt-in — refusing it would block live DR).

use super::env::{existing_compose_files, ShopEnv};
use super::error::Error;
use super::live::LiveSignals;
use super::mysql::{compose_argv, compose_cli_log, require_docker};
use std::path::Path;
use std::process::Command;

pub const DOMAIN_COMMAND: &str = "sales-channel:update:domain";

/// Shown when the console command exits non-zero. Shopware ships this command.
pub const UPDATE_DOMAIN_FAILED: &str = "sales-channel:update:domain failed.";

pub fn rewrite_app_url(env: &ShopEnv) -> Option<&str> {
    env.get("APP_URL")
}

pub fn rewrite_requested(env: &ShopEnv) -> bool {
    rewrite_app_url(env).is_some()
}

/// Domain update never runs on live. Returns Ok so restore/DR can continue.
pub fn assert_not_live_rewrite(signals: &LiveSignals) -> Result<(), Error> {
    if !signals.is_live() {
        return Ok(());
    }
    Ok(())
}

pub fn should_rewrite(env: &ShopEnv, signals: &LiveSignals) -> bool {
    rewrite_requested(env) && !signals.is_live()
}

/// Host argument for `sales-channel:update:domain`.
///
/// Scheme, userinfo, port, path, query, and fragment are not passed.
/// `https://staging.example.com` and `https://staging.example.com:8443/en`
/// both become `staging.example.com`.
pub fn app_url_host(app_url: &str) -> Option<String> {
    let url = app_url.trim();
    if url.is_empty() {
        return None;
    }
    let after_scheme = match url.split_once("://") {
        Some((scheme, rest)) => {
            if scheme.is_empty() || rest.is_empty() {
                return None;
            }
            rest
        }
        None => url,
    };
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        return None;
    }
    let authority = match authority.rsplit_once('@') {
        Some((_, hostport)) => hostport,
        None => authority,
    };
    if authority.is_empty() {
        return None;
    }
    let host = if let Some(rest) = authority.strip_prefix('[') {
        let end = rest.find(']')?;
        let host = &rest[..end];
        if host.is_empty() {
            return None;
        }
        host
    } else {
        match authority.split_once(':') {
            Some((host, _)) => host,
            None => authority,
        }
    };
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

pub fn domain_host(env: &ShopEnv) -> Result<String, Error> {
    let url = rewrite_app_url(env)
        .ok_or_else(|| Error::fail("APP_URL is unset; cannot run sales-channel:update:domain."))?;
    app_url_host(url).ok_or_else(|| {
        Error::fail(format!(
            "APP_URL has no host for sales-channel:update:domain ({url}). The argument is the host only: scheme and path are stripped, and the port is not passed."
        ))
    })
}

fn push_domain_command(args: &mut Vec<String>, host: &str) {
    args.extend([
        "run".into(),
        "--rm".into(),
        "--pull".into(),
        "never".into(),
        "--entrypoint".into(),
        "php".into(),
        "web".into(),
        "bin/console".into(),
        DOMAIN_COMMAND.into(),
    ]);
    args.push(host.to_string());
}

pub fn compose_rewrite_args(env: &ShopEnv, compose_dir: &Path) -> Result<Vec<String>, Error> {
    let files = existing_compose_files(compose_dir);
    let host = domain_host(env)?;
    let mut args = compose_argv(compose_dir, &files);
    push_domain_command(&mut args, &host);
    Ok(args)
}

pub fn rewrite_log_line(env: &ShopEnv, compose_dir: &Path) -> Result<String, Error> {
    let files = existing_compose_files(compose_dir);
    let host = domain_host(env)?;
    Ok(format!(
        "{} run --rm --pull never --entrypoint php web bin/console {DOMAIN_COMMAND} {host}",
        compose_cli_log(compose_dir, &files),
    ))
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
        "==> Sales_channel_domain rewrite via {DOMAIN_COMMAND} from APP_URL (sales channel domains only; media CDN / plugin configs / payment webhooks are not updated)"
    );
    if files.is_empty() {
        return Err(Error::fail(
            "No compose files found under shop root; cannot run sales-channel:update:domain.",
        ));
    }
    let args = compose_rewrite_args(env, compose_dir)?;
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
        return Err(Error::fail(UPDATE_DOMAIN_FAILED));
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

    fn assert_no_legacy_flags(joined: &str) {
        for gone in [
            "--app-url",
            "--deploy-env",
            "--sync-env",
            "--checkout-basename",
            "--dry-run",
            "--map",
            "sales-channel:replace:url",
            "fyrst:sales-channel:rewrite-urls",
            "composer update fyrst/shopware-cd",
        ] {
            assert!(!joined.contains(gone), "{gone} in {joined}");
        }
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
    fn host_strips_scheme_path_and_port() {
        assert_eq!(
            app_url_host("https://staging.example.com").as_deref(),
            Some("staging.example.com")
        );
        assert_eq!(
            app_url_host("https://staging.example.com/").as_deref(),
            Some("staging.example.com")
        );
        assert_eq!(
            app_url_host("https://staging.example.com/de").as_deref(),
            Some("staging.example.com")
        );
        assert_eq!(
            app_url_host("https://staging.example.com:8443/en?x=1").as_deref(),
            Some("staging.example.com")
        );
        assert_eq!(
            app_url_host("http://localhost:8000").as_deref(),
            Some("localhost")
        );
        assert_eq!(
            app_url_host("https://user:pass@staging.example.com:8443/shop").as_deref(),
            Some("staging.example.com")
        );
        assert_eq!(app_url_host("  ").as_deref(), None);
        assert_eq!(app_url_host("https://").as_deref(), None);
    }

    #[test]
    fn console_args_are_command_plus_host_only() {
        let env = env_from(&[
            ("APP_URL", "https://staging.example.com:8443/de"),
            ("SHOPWARE_DEPLOY_ENV", "staging"),
        ]);
        let args = compose_rewrite_args(&env, Path::new("/shops/acme-staging")).unwrap();
        let joined = args.join(" ");
        let cmd_at = args
            .iter()
            .position(|a| a == DOMAIN_COMMAND)
            .expect(&joined);
        assert!(cmd_at > 0, "{joined}");
        assert_eq!(args[cmd_at - 1], "bin/console");
        assert_eq!(args[cmd_at + 1], "staging.example.com");
        assert_eq!(args.len(), cmd_at + 2, "{joined}");
        assert!(
            joined.contains("run --rm --pull never --entrypoint php web bin/console"),
            "{joined}"
        );
        assert!(!joined.contains(":8443"), "{joined}");
        assert!(!joined.contains("https://"), "{joined}");
        assert!(!joined.contains("/de"), "{joined}");
        assert_no_legacy_flags(&joined);
    }

    #[test]
    fn failure_text_does_not_ask_for_shopware_cd_update() {
        assert!(!UPDATE_DOMAIN_FAILED.contains("composer update fyrst/shopware-cd"));
        assert!(!UPDATE_DOMAIN_FAILED.contains("fyrst:sales-channel:rewrite-urls"));
        assert!(UPDATE_DOMAIN_FAILED.contains(DOMAIN_COMMAND));
    }

    #[test]
    fn compose_line_is_console_not_sql() {
        let env = env_from(&[
            ("APP_URL", "https://staging.example.com"),
            ("SHOPWARE_SHOP_ID", "acme"),
            ("SHOPWARE_DEPLOY_ENV", "staging"),
            ("COMPOSE_PROJECT_NAME", "shopware-acme"),
        ]);
        let line = rewrite_log_line(&env, Path::new("/shops/acme-staging")).unwrap();
        assert!(line.contains(DOMAIN_COMMAND), "{line}");
        assert!(line.contains(" staging.example.com"), "{line}");
        assert!(
            line.contains("run --rm --pull never --entrypoint php"),
            "{line}"
        );
        assert!(line.contains("--env-file .env"), "{line}");
        assert!(
            !line
                .split_whitespace()
                .any(|t| t == "-p" || t == "--project-name"),
            "{line}"
        );
        assert!(!line.contains("shopware-acme"), "{line}");
        assert!(
            !line.to_ascii_lowercase().contains("update sales_channel"),
            "{line}"
        );
        assert_no_legacy_flags(&line);
        let args = compose_rewrite_args(&env, Path::new("/shops/acme-staging")).unwrap();
        assert!(args.contains(&DOMAIN_COMMAND.to_string()));
        assert_eq!(args.last().map(String::as_str), Some("staging.example.com"));
        assert!(!args.iter().any(|a| a == "-p" || a == "--project-name"));
    }

    #[test]
    fn rewrite_argv_includes_host_env_files() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("fyrst-cli-rew-env-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(dir.join("deploy")).unwrap();
        std::fs::write(
            dir.join("deploy/compose.yaml"),
            "services:\n  web:\n    image: x\n",
        )
        .unwrap();
        std::fs::write(dir.join(".env.local"), "SHOPWARE_DEPLOY_ENV=staging\n").unwrap();
        let env = env_from(&[
            ("APP_URL", "https://staging.example.com"),
            ("SHOPWARE_SHOP_ID", "acme"),
            ("SHOPWARE_DEPLOY_ENV", "staging"),
        ]);
        let args = compose_rewrite_args(&env, &dir).unwrap();
        assert!(args.windows(2).any(|w| w == ["--env-file", ".env"]));
        assert!(args.windows(2).any(|w| w == ["--env-file", ".env.local"]));
        assert!(args.windows(2).any(|w| w == ["-f", "deploy/compose.yaml"]));
        assert!(!args.iter().any(|a| a == "-p" || a == "--project-name"));
        assert_eq!(args.last().map(String::as_str), Some("staging.example.com"));
        let _ = std::fs::remove_dir_all(&dir);
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
