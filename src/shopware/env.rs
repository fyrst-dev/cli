//! Shop-root discovery, `.env` loading, and Compose project name.
//!
//! One loader for every shopware verb: process env is the base, then
//! `.env`, `.env.local` (if present), `.env.prod` (if present). Later files
//! win except identity / process-win keys, which a non-empty process value
//! still owns. Empty `SHOPWARE_DEPLOY_ENV=` / `COMPOSE_PROJECT_NAME=` in a file
//! is ignored so a host file (`.env.local` / `.env.prod`) can own the value
//! over an empty leftover in committed `.env`. Leftover `SYNC_*` /
//! `BACKUP_SSH_*` / `BACKUP_ALLOW_*` names fail when the replacement is unset.

use super::envfile::parse_env_file;
use super::error::Error;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Keys a non-empty process value keeps after files load (same safety as
/// today's identity / VPS / backup win-lists, new names only).
const PROCESS_WINS: &[&str] = &[
    "SHOPWARE_SHOP_ID",
    "SHOPWARE_DEPLOY_ENV",
    "IMAGE",
    "IMAGE_TAG",
    "COMPOSE_PROJECT_NAME",
    "SHOPWARE_DATA_ROOT",
    "SHOPWARE_DATA_BASE",
    "SHOPWARE_REMOTE_DATA_ROOT",
    "SHOPWARE_SSH_HOST",
    "SHOPWARE_SSH_USER",
    "SHOPWARE_SSH_KEY",
    "SHOPWARE_ALLOW_LIVE_RESTORE",
    "APP_URL",
    "DEPLOY_HEALTH_URL",
    "ALLOW_NO_DEPLOY_HEALTH",
    "COMPOSE_PROFILES",
    "ROLLBACK_ON_FAIL",
    "SKIP_PULL",
    "PULL_POLICY",
    "BACKUP_TARGET",
    "BACKUP_KEEP_DAYS",
    "BACKUP_DB_DUMP",
    "BACKUP_CONFIRM_RESTORE",
];

/// Root `.env.*` only. Later file wins. Never `deploy/*.env`.
/// Same order as `docker compose --env-file` flags.
pub const ENV_FILES: &[&str] = &[".env", ".env.local", ".env.prod"];

/// `--env-file` flags: always `.env`, then `.env.local` / `.env.prod` if present.
pub fn compose_env_file_flags(compose_dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for rel in ENV_FILES {
        if *rel != ".env" && !compose_dir.join(rel).is_file() {
            continue;
        }
        out.push("--env-file".into());
        out.push((*rel).to_string());
    }
    out
}

pub const COMPOSE_FILES: &[&str] = &[
    "deploy/compose.yaml",
    "deploy/compose.prod.yaml",
    "deploy/compose.vps.yaml",
];

pub const DEFAULT_DATA_BASE: &str = "/var/lib/shopware/data";
pub const DEFAULT_REMOTE_ENV: &str = "live";
pub const DEFAULT_ARCHIVE_IMAGE: &str = "alpine:3.20";
pub const DEFAULT_BACKUP_TARGET: &str = "local";

const EXACT_LEGACY: &[(&str, &str)] = &[
    ("SYNC_ENV", "SHOPWARE_DEPLOY_ENV"),
    ("SYNC_DATA_ROOT", "SHOPWARE_DATA_ROOT"),
    ("SYNC_SSH_HOST", "SHOPWARE_SSH_HOST"),
    ("SYNC_SSH_USER", "SHOPWARE_SSH_USER"),
    ("SYNC_SSH_KEY", "SHOPWARE_SSH_KEY"),
    ("SYNC_ALLOW_LIVE_RESTORE", "SHOPWARE_ALLOW_LIVE_RESTORE"),
    ("SYNC_APP_URL", "APP_URL"),
    ("SYNC_REWRITE_APP_URL", "APP_URL"),
    ("SYNC_REWRITE_URL_MAP", "APP_URL"),
    ("SYNC_REMOTE_DATA_ROOT", "SHOPWARE_REMOTE_DATA_ROOT"),
    ("SYNC_SOURCE_ENV", "SHOPWARE_REMOTE_DATA_ROOT"),
    ("BACKUP_SSH_HOST", "SHOPWARE_SSH_HOST"),
    ("BACKUP_SSH_USER", "SHOPWARE_SSH_USER"),
    ("BACKUP_SSH_KEY", "SHOPWARE_SSH_KEY"),
    ("BACKUP_SSH_PORT", "SHOPWARE_SSH_HOST"),
    ("SYNC_SSH_PORT", "SHOPWARE_SSH_HOST"),
    ("BACKUP_ALLOW_LIVE_RESTORE", "SHOPWARE_ALLOW_LIVE_RESTORE"),
];

#[derive(Debug)]
pub struct ShopEnv {
    pub compose_dir: PathBuf,
    vars: HashMap<String, String>,
}

impl ShopEnv {
    /// Load shop-root `.env` / `.env.local` / `.env.prod` for every shopware verb.
    pub fn load(project_root: PathBuf, process: &HashMap<String, String>) -> Result<Self, Error> {
        let mut vars = process.clone();
        for rel in ENV_FILES {
            let path = project_root.join(rel);
            if path.is_file() {
                let contents = fs::read_to_string(&path)
                    .map_err(|e| Error::fail(format!("cannot read {}: {e}", path.display())))?;
                for (k, v) in parse_env_file(&contents) {
                    if matches!(k.as_str(), "SHOPWARE_DEPLOY_ENV" | "COMPOSE_PROJECT_NAME")
                        && v.is_empty()
                    {
                        // Empty leftover in shared `.env` must not hide a host file.
                        continue;
                    }
                    vars.insert(k, v);
                }
            }
        }
        for key in PROCESS_WINS {
            if let Some(preset) = process.get(*key).filter(|s| !s.is_empty()) {
                vars.insert((*key).to_string(), preset.clone());
            }
        }
        reject_legacy_names(&vars)?;
        Ok(Self {
            compose_dir: project_root,
            vars,
        })
    }

    /// Non-empty value, same idea as bash `${VAR:-}` for required checks.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars
            .get(key)
            .map(String::as_str)
            .filter(|s| !s.is_empty())
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.vars.insert(key.into(), value.into());
    }

    #[cfg(test)]
    pub fn from_vars(compose_dir: PathBuf, vars: HashMap<String, String>) -> Self {
        Self { compose_dir, vars }
    }
}

fn is_set(vars: &HashMap<String, String>, key: &str) -> bool {
    vars.get(key).map(|s| !s.is_empty()).unwrap_or(false)
}

fn replacement_for(key: &str) -> Option<&'static str> {
    for (old, new) in EXACT_LEGACY {
        if *old == key {
            return Some(*new);
        }
    }
    if key.starts_with("BACKUP_ALLOW_") {
        return Some("SHOPWARE_ALLOW_LIVE_RESTORE");
    }
    if key.starts_with("BACKUP_SSH_") {
        return match key {
            "BACKUP_SSH_KEY" => Some("SHOPWARE_SSH_KEY"),
            "BACKUP_SSH_USER" => Some("SHOPWARE_SSH_USER"),
            _ => Some("SHOPWARE_SSH_HOST"),
        };
    }
    let rest = key.strip_prefix("SYNC_")?;
    if rest.ends_with("_SSH_HOST") || rest == "SSH_HOST" {
        return Some("SHOPWARE_SSH_HOST");
    }
    if rest.ends_with("_SSH_USER") || rest == "SSH_USER" {
        return Some("SHOPWARE_SSH_USER");
    }
    if rest.ends_with("_SSH_KEY") || rest == "SSH_KEY" {
        return Some("SHOPWARE_SSH_KEY");
    }
    if rest.ends_with("_DATA_ROOT") {
        return Some("SHOPWARE_REMOTE_DATA_ROOT");
    }
    None
}

/// Fail when an old name is set and its replacement is unset. No silent aliases.
pub fn reject_legacy_names(vars: &HashMap<String, String>) -> Result<(), Error> {
    let mut missing: Vec<(&str, String)> = Vec::new();
    for key in vars.keys() {
        if !is_set(vars, key) {
            continue;
        }
        let Some(new) = replacement_for(key) else {
            continue;
        };
        if !is_set(vars, new) && !missing.iter().any(|(n, old)| *n == new && old == key) {
            missing.push((new, key.clone()));
        }
    }
    missing.sort_by(|a, b| a.0.cmp(b.0).then(a.1.cmp(&b.1)));
    missing.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    if missing.is_empty() {
        return Ok(());
    }
    let detail = missing
        .iter()
        .map(|(new, old)| format!("{new} (found leftover {old})"))
        .collect::<Vec<_>>()
        .join("; ");
    Err(Error::fail(format!(
        "{detail} is required. Leftover SYNC_*/BACKUP_SSH_*/BACKUP_ALLOW_* names are no longer read."
    )))
}

pub fn resolve_compose_dir(
    process: &HashMap<String, String>,
    cwd: &Path,
) -> Result<PathBuf, Error> {
    if let Some(dir) = process.get("COMPOSE_DIR").filter(|s| !s.is_empty()) {
        let p = PathBuf::from(dir);
        let p = if p.is_absolute() { p } else { cwd.join(p) };
        return require_shop_root(&p);
    }
    let mut dir = cwd.to_path_buf();
    loop {
        if looks_like_shop_root(&dir) {
            return Ok(dir);
        }
        if !dir.pop() {
            break;
        }
    }
    if cwd.join(".env").is_file() {
        return Ok(cwd.to_path_buf());
    }
    Err(Error::fail(format!(
        "Cannot find shop-root .env (cwd={}, COMPOSE_DIR unset). Set COMPOSE_DIR or run from the shop checkout.",
        cwd.display()
    )))
}

fn has_deploy_marker(dir: &Path) -> bool {
    dir.join("deploy/compose.yaml").is_file()
        || dir.join("deploy/compose.yml").is_file()
        || dir.join("deploy").is_dir()
}

pub fn looks_like_shop_root(dir: &Path) -> bool {
    dir.join(".env").is_file() && has_deploy_marker(dir)
}

/// `env init` may run before `.env` exists (copy from `.env.example`).
pub fn looks_like_shop_root_init(dir: &Path) -> bool {
    (dir.join(".env").is_file() || dir.join(".env.example").is_file()) && has_deploy_marker(dir)
}

/// Shop checkout for `env init`. Unlike [`resolve_compose_dir`], `.env` may still
/// be missing when `.env.example` is present. `COMPOSE_DIR` need only be a directory.
pub fn resolve_compose_dir_init(
    process: &HashMap<String, String>,
    cwd: &Path,
) -> Result<PathBuf, Error> {
    if let Some(dir) = process.get("COMPOSE_DIR").filter(|s| !s.is_empty()) {
        let p = PathBuf::from(dir);
        let p = if p.is_absolute() { p } else { cwd.join(p) };
        if !p.is_dir() {
            return Err(Error::fail(format!("Cannot cd to COMPOSE_DIR={dir}")));
        }
        return Ok(fs::canonicalize(&p).unwrap_or(p));
    }
    let mut dir = cwd.to_path_buf();
    loop {
        if looks_like_shop_root_init(&dir) {
            return Ok(fs::canonicalize(&dir).unwrap_or(dir));
        }
        if !dir.pop() {
            break;
        }
    }
    if cwd.join(".env").is_file() || cwd.join(".env.example").is_file() {
        return Ok(fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf()));
    }
    Err(Error::fail(format!(
        "Cannot find shop-root .env or .env.example (cwd={}, COMPOSE_DIR unset). Set COMPOSE_DIR or run from the shop checkout.",
        cwd.display()
    )))
}

fn require_shop_root(p: &Path) -> Result<PathBuf, Error> {
    if !p.is_dir() {
        return Err(Error::fail(format!(
            "COMPOSE_DIR is not a directory: {}",
            p.display()
        )));
    }
    if !p.join(".env").is_file() {
        return Err(Error::fail(format!(
            "Shop root is missing .env (COMPOSE_DIR={}).",
            p.display()
        )));
    }
    Ok(fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
}

pub fn require_shop_id(env: &ShopEnv) -> Result<String, Error> {
    env.get("SHOPWARE_SHOP_ID")
        .map(str::to_string)
        .ok_or_else(|| {
            Error::fail(
                "SHOPWARE_SHOP_ID is required (stable shop slug, same on live + staging + laptop). Set it in shop-root .env.",
            )
        })
}

pub fn require_deploy_env(env: &ShopEnv) -> Result<String, Error> {
    env.get("SHOPWARE_DEPLOY_ENV")
        .map(str::to_string)
        .ok_or_else(|| {
            Error::fail(
                "SHOPWARE_DEPLOY_ENV is required (live|staging|playground|dev). Set it in .env.local (host-specific; not committed shared .env).",
            )
        })
}

/// Overlay `vps_env_truthy`: `1` / `true` / `yes` / `on` (case-insensitive).
pub fn env_truthy(value: Option<&str>) -> bool {
    match value.map(str::trim).filter(|s| !s.is_empty()) {
        Some(v) => matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        None => false,
    }
}

pub fn allow_live_restore(env: &ShopEnv) -> bool {
    env_truthy(env.get("SHOPWARE_ALLOW_LIVE_RESTORE"))
}

/// Compose project `{SHOPWARE_SHOP_ID}-{SHOPWARE_DEPLOY_ENV}`.
///
/// Same string locally (`.env.local` + `compose.override.yaml` `name:`) and
/// on VPS (`deploy/compose.yaml` `name:` interpolating host env files). Do
/// not store this (or `COMPOSE_PROJECT_NAME`) in committed `.env`.
pub fn vps_project_name(shop_id: &str, deploy_env: &str) -> String {
    format!("{shop_id}-{deploy_env}")
}

/// Derived project name when shop id and deploy env are both set.
pub fn vps_project_name_opt(env: &ShopEnv) -> Option<String> {
    match (env.get("SHOPWARE_SHOP_ID"), env.get("SHOPWARE_DEPLOY_ENV")) {
        (Some(id), Some(deploy_env)) if !id.is_empty() && !deploy_env.is_empty() => {
            Some(vps_project_name(id, deploy_env))
        }
        _ => None,
    }
}

/// `(project_name, derived_from_shop_id_and_env)`.
///
/// Prefers `{SHOPWARE_SHOP_ID}-{SHOPWARE_DEPLOY_ENV}` so named-volume fallback
/// matches compose `name:`. `COMPOSE_PROJECT_NAME` is a last-resort
/// fallback when shop id / deploy env are missing.
pub fn derive_project_name(env: &ShopEnv) -> Result<(String, bool), Error> {
    if let Some(n) = vps_project_name_opt(env) {
        return Ok((n, true));
    }
    if let Some(n) = env.get("COMPOSE_PROJECT_NAME") {
        return Ok((n.to_string(), false));
    }
    Err(Error::fail(
        "Set SHOPWARE_SHOP_ID in shared .env and SHOPWARE_DEPLOY_ENV in .env.local to derive ${SHOPWARE_SHOP_ID}-${SHOPWARE_DEPLOY_ENV} for Compose (deploy/compose.yaml name: + host env files).",
    ))
}

/// Work directory for sync capture/apply (`--snapshot-dir`, else `var/runtime-sync`).
pub fn resolve_snapshot_dir(cli_dir: Option<&str>, _env: &ShopEnv, compose_dir: &Path) -> PathBuf {
    if let Some(d) = cli_dir.map(str::trim).filter(|s| !s.is_empty()) {
        let p = PathBuf::from(d);
        if p.is_absolute() {
            p
        } else {
            compose_dir.join(p)
        }
    } else {
        compose_dir.join("var/runtime-sync")
    }
}

pub fn data_base(env: &ShopEnv) -> &str {
    env.get("SHOPWARE_DATA_BASE").unwrap_or(DEFAULT_DATA_BASE)
}

pub fn archive_image(_env: &ShopEnv) -> String {
    DEFAULT_ARCHIVE_IMAGE.to_string()
}

pub fn derived_data_root(env: &ShopEnv, shop_id: &str, deploy_env: &str) -> PathBuf {
    PathBuf::from(data_base(env)).join(shop_id).join(deploy_env)
}

/// Bind-mount root on this host (`SHOPWARE_DATA_ROOT` or identity formula).
pub fn derive_local_data_root(env: &ShopEnv) -> Result<PathBuf, Error> {
    Ok(local_data_root(env)?.0)
}

/// Local bind-mount root plus whether the path was derived.
pub fn local_data_root(env: &ShopEnv) -> Result<(PathBuf, bool), Error> {
    if let Some(p) = env.get("SHOPWARE_DATA_ROOT") {
        return Ok((PathBuf::from(p), false));
    }
    let shop_id = require_shop_id(env)?;
    let deploy_env = require_deploy_env(env)?;
    Ok((derived_data_root(env, &shop_id, &deploy_env), true))
}

/// Alias used by `sync apply` (same resolution as capture).
pub fn resolve_data_root(env: &ShopEnv) -> Result<PathBuf, Error> {
    derive_local_data_root(env)
}

/// Backup bind-mount root: same formula as [`local_data_root`].
pub fn resolve_backup_data_root(env: &ShopEnv, shop_id: &str, deploy_env: &str) -> (PathBuf, bool) {
    if let Some(root) = env.get("SHOPWARE_DATA_ROOT") {
        return (PathBuf::from(root), false);
    }
    (derived_data_root(env, shop_id, deploy_env), true)
}

/// Remote live data: `SHOPWARE_REMOTE_DATA_ROOT` or `{base}/{shop_id}/live`.
pub fn remote_data_root(env: &ShopEnv, shop_id: &str) -> PathBuf {
    if let Some(p) = env.get("SHOPWARE_REMOTE_DATA_ROOT") {
        return PathBuf::from(p);
    }
    derived_data_root(env, shop_id, DEFAULT_REMOTE_ENV)
}

pub fn have_cmd(name: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| dir.join(name).is_file())
}

pub fn require_cmd(name: &str) -> Result<(), Error> {
    if have_cmd(name) {
        Ok(())
    } else {
        Err(Error::fail(format!(
            "Missing command '{name}'. Install docker, bash, openssh-client, gzip, and rsync on this host."
        )))
    }
}

pub fn existing_compose_files(compose_dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for rel in COMPOSE_FILES {
        if compose_dir.join(rel).is_file() {
            files.push((*rel).to_string());
        }
    }
    if files.is_empty() {
        for rel in ["deploy/compose.yml", "compose.yaml", "compose.yml"] {
            if compose_dir.join(rel).is_file() {
                files.push(rel.to_string());
            }
        }
    }
    files
}

pub fn compose_mentions_mysql_service(compose_dir: &Path) -> bool {
    let mut paths: Vec<PathBuf> = existing_compose_files(compose_dir)
        .into_iter()
        .map(|rel| compose_dir.join(rel))
        .collect();
    for rel in [
        "deploy/compose.yaml",
        "deploy/compose.yml",
        "compose.yaml",
        "compose.yml",
    ] {
        let p = compose_dir.join(rel);
        if p.is_file() && !paths.contains(&p) {
            paths.push(p);
        }
    }
    for p in paths {
        if let Ok(s) = fs::read_to_string(&p) {
            if yaml_has_service(&s, "mysql") {
                return true;
            }
        }
    }
    false
}

/// Heuristic: `mysql:` as a service key (indent 2 or 4) under `services:`.
/// Ignores nested `depends_on: mysql:` (deeper indent).
pub fn yaml_has_service(contents: &str, name: &str) -> bool {
    let mut in_services = false;
    for line in contents.lines() {
        let indent = line.chars().take_while(|c| *c == ' ').count();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if indent == 0 && trimmed.starts_with("services:") {
            in_services = true;
            continue;
        }
        if indent == 0 && trimmed.ends_with(':') {
            in_services = trimmed.starts_with("services:");
            continue;
        }
        if in_services && (indent == 2 || indent == 4) && trimmed.starts_with(name) {
            let rest = &trimmed[name.len()..];
            if rest.starts_with(':') {
                return true;
            }
        }
    }
    false
}

pub fn is_local_source(from: Option<&str>) -> bool {
    match from.map(str::trim).filter(|s| !s.is_empty()) {
        None => true,
        Some(s) => {
            let l = s.to_ascii_lowercase();
            l == "local" || l == "this"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_shop(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p =
            std::env::temp_dir().join(format!("fyrst-cli-{prefix}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(p.join("deploy")).unwrap();
        p
    }

    #[test]
    fn yaml_mysql_service_not_depends_on() {
        let y = "\
services:
    web:
        depends_on:
            mysql:
                condition: service_healthy
    mysql:
        image: mysql:8.4
";
        assert!(yaml_has_service(y, "mysql"));
        let no = "\
services:
    web:
        depends_on:
            mysql:
                condition: service_healthy
";
        assert!(!yaml_has_service(no, "mysql"));
    }

    #[test]
    fn process_identity_overrides_file() {
        let shop = temp_shop("env-override");
        fs::write(
            shop.join(".env"),
            "SHOPWARE_SHOP_ID=fromfile\nSHOPWARE_DEPLOY_ENV=staging\nMYSQL_USER=shop\n",
        )
        .unwrap();
        let mut process = HashMap::new();
        process.insert("SHOPWARE_SHOP_ID".into(), "fromproc".into());
        let env = ShopEnv::load(shop.clone(), &process).unwrap();
        assert_eq!(env.get("SHOPWARE_SHOP_ID"), Some("fromproc"));
        assert_eq!(env.get("SHOPWARE_DEPLOY_ENV"), Some("staging"));
        assert_eq!(env.get("MYSQL_USER"), Some("shop"));
        let _ = fs::remove_dir_all(&shop);
    }

    #[test]
    fn later_file_wins_except_process_identity() {
        let shop = temp_shop("later-file");
        fs::write(
            shop.join(".env"),
            "SHOPWARE_SHOP_ID=fromenv\nAPP_URL=http://from-env\nBACKUP_TARGET=/from-env\n",
        )
        .unwrap();
        fs::write(
            shop.join(".env.local"),
            "APP_URL=http://from-local\nSHOPWARE_SSH_HOST=vps.local\nBACKUP_TARGET=/from-local\n",
        )
        .unwrap();
        fs::write(
            shop.join(".env.prod"),
            "APP_URL=http://from-prod\nDEPLOY_HEALTH_URL=http://health\n",
        )
        .unwrap();
        let mut process = HashMap::new();
        process.insert("SHOPWARE_SHOP_ID".into(), "fromproc".into());
        let env = ShopEnv::load(shop.clone(), &process).unwrap();
        assert_eq!(env.get("SHOPWARE_SHOP_ID"), Some("fromproc"));
        assert_eq!(env.get("APP_URL"), Some("http://from-prod"));
        assert_eq!(env.get("SHOPWARE_SSH_HOST"), Some("vps.local"));
        assert_eq!(env.get("BACKUP_TARGET"), Some("/from-local"));
        assert_eq!(env.get("DEPLOY_HEALTH_URL"), Some("http://health"));
        let _ = fs::remove_dir_all(&shop);
    }

    #[test]
    fn deploy_sync_env_is_not_loaded() {
        let shop = temp_shop("ignore-sync-env");
        fs::write(
            shop.join(".env"),
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=staging\n",
        )
        .unwrap();
        fs::create_dir_all(shop.join("deploy")).unwrap();
        fs::write(
            shop.join("deploy/sync.env"),
            "SHOPWARE_SHOP_ID=from-sync-env\nSHOPWARE_SSH_HOST=should-not-load\n",
        )
        .unwrap();
        fs::write(
            shop.join("deploy/backup.env"),
            "BACKUP_TARGET=/should-not-load\n",
        )
        .unwrap();
        let env = ShopEnv::load(shop.clone(), &HashMap::new()).unwrap();
        assert_eq!(env.get("SHOPWARE_SHOP_ID"), Some("acme"));
        assert!(env.get("SHOPWARE_SSH_HOST").is_none());
        assert!(env.get("BACKUP_TARGET").is_none());
        let _ = fs::remove_dir_all(&shop);
    }

    #[test]
    fn vps_ci_image_tag_wins_over_env_latest() {
        let shop = temp_shop("vps-ci-tag");
        fs::write(
            shop.join(".env"),
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=live
IMAGE=ghcr.io/from-file/shop
IMAGE_TAG=latest
DEPLOY_HEALTH_URL=http://from-file-health
COMPOSE_PROFILES=redis
",
        )
        .unwrap();
        let mut process = HashMap::new();
        process.insert("IMAGE".into(), "ghcr.io/from-ci/shop".into());
        process.insert("IMAGE_TAG".into(), "abc123deadbeef".into());
        process.insert(
            "DEPLOY_HEALTH_URL".into(),
            "http://127.0.0.1:8000/api/_info/health-check".into(),
        );
        let env = ShopEnv::load(shop.clone(), &process).unwrap();
        assert_eq!(env.get("IMAGE"), Some("ghcr.io/from-ci/shop"));
        assert_eq!(env.get("IMAGE_TAG"), Some("abc123deadbeef"));
        assert_eq!(
            env.get("DEPLOY_HEALTH_URL"),
            Some("http://127.0.0.1:8000/api/_info/health-check")
        );
        assert_eq!(env.get("COMPOSE_PROFILES"), Some("redis"));
        let _ = fs::remove_dir_all(&shop);
    }

    #[test]
    fn leftover_sync_ssh_fails_without_replacement() {
        let shop = temp_shop("legacy-ssh");
        fs::write(
            shop.join(".env"),
            "SHOPWARE_SHOP_ID=acme\nSYNC_SSH_HOST=vps.example\n",
        )
        .unwrap();
        let err = ShopEnv::load(shop.clone(), &HashMap::new()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("SHOPWARE_SSH_HOST"), "{msg}");
        assert!(msg.contains("SYNC_SSH_HOST"), "{msg}");
        let _ = fs::remove_dir_all(&shop);
    }

    #[test]
    fn leftover_sync_ssh_ok_when_replacement_set() {
        let shop = temp_shop("legacy-ssh-ok");
        fs::write(
            shop.join(".env"),
            "SHOPWARE_SHOP_ID=acme\nSYNC_SSH_HOST=old.example\nSHOPWARE_SSH_HOST=new.example\n",
        )
        .unwrap();
        let env = ShopEnv::load(shop.clone(), &HashMap::new()).unwrap();
        assert_eq!(env.get("SHOPWARE_SSH_HOST"), Some("new.example"));
        let _ = fs::remove_dir_all(&shop);
    }

    #[test]
    fn leftover_sync_app_url_fails_without_app_url() {
        let shop = temp_shop("legacy-app-url");
        fs::write(
            shop.join(".env"),
            "SHOPWARE_SHOP_ID=acme\nSYNC_APP_URL=https://old.example\n",
        )
        .unwrap();
        let err = ShopEnv::load(shop.clone(), &HashMap::new()).unwrap_err();
        assert!(err.to_string().contains("APP_URL"), "{err}");
        let _ = fs::remove_dir_all(&shop);
    }

    #[test]
    fn leftover_backup_allow_fails_without_shopware_allow() {
        let shop = temp_shop("legacy-allow");
        fs::write(
            shop.join(".env"),
            "SHOPWARE_SHOP_ID=acme\nBACKUP_ALLOW_LIVE_RESTORE=1\n",
        )
        .unwrap();
        let err = ShopEnv::load(shop.clone(), &HashMap::new()).unwrap_err();
        assert!(
            err.to_string().contains("SHOPWARE_ALLOW_LIVE_RESTORE"),
            "{err}"
        );
        let _ = fs::remove_dir_all(&shop);
    }

    #[test]
    fn leftover_alias_ssh_fails_with_shopware_ssh() {
        let shop = temp_shop("legacy-alias");
        fs::write(
            shop.join(".env"),
            "SHOPWARE_SHOP_ID=acme\nSYNC_LIVE_SSH_USER=deploy\n",
        )
        .unwrap();
        let err = ShopEnv::load(shop.clone(), &HashMap::new()).unwrap_err();
        assert!(err.to_string().contains("SHOPWARE_SSH_USER"), "{err}");
        let _ = fs::remove_dir_all(&shop);
    }

    #[test]
    fn file_mysql_password_not_logged_in_map_get_empty() {
        let shop = temp_shop("env-empty");
        let mut f = fs::File::create(shop.join(".env")).unwrap();
        writeln!(f, "SHOPWARE_SHOP_ID=acme").unwrap();
        writeln!(f, "MYSQL_PASSWORD=").unwrap();
        drop(f);
        let env = ShopEnv::load(shop.clone(), &HashMap::new()).unwrap();
        assert!(env.get("MYSQL_PASSWORD").is_none());
        let _ = fs::remove_dir_all(&shop);
    }

    #[test]
    fn derived_project_name() {
        let mut vars = HashMap::new();
        vars.insert("SHOPWARE_SHOP_ID".into(), "acme".into());
        vars.insert("SHOPWARE_DEPLOY_ENV".into(), "live".into());
        let env = ShopEnv::from_vars(PathBuf::from("/tmp/x"), vars);
        assert_eq!(
            derive_project_name(&env).unwrap(),
            ("acme-live".into(), true)
        );
    }

    #[test]
    fn compose_project_name_does_not_override_identity_name() {
        let mut vars = HashMap::new();
        vars.insert("COMPOSE_PROJECT_NAME".into(), "sw-shop-acme".into());
        vars.insert("SHOPWARE_SHOP_ID".into(), "acme".into());
        vars.insert("SHOPWARE_DEPLOY_ENV".into(), "live".into());
        let env = ShopEnv::from_vars(PathBuf::from("/tmp/x"), vars);
        assert_eq!(
            derive_project_name(&env).unwrap(),
            ("acme-live".into(), true)
        );
        assert_eq!(vps_project_name_opt(&env).as_deref(), Some("acme-live"));
    }

    #[test]
    fn compose_project_name_fallback_without_shop_identity() {
        let mut vars = HashMap::new();
        vars.insert("COMPOSE_PROJECT_NAME".into(), "sw-shop-acme".into());
        let env = ShopEnv::from_vars(PathBuf::from("/tmp/x"), vars);
        assert_eq!(
            derive_project_name(&env).unwrap(),
            ("sw-shop-acme".into(), false)
        );
        assert_eq!(vps_project_name_opt(&env), None);
    }

    #[test]
    fn deploy_env_from_local_wins_over_shared_env() {
        let shop = temp_shop("deploy-env-local-wins");
        fs::write(
            shop.join(".env"),
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=live\n",
        )
        .unwrap();
        fs::write(shop.join(".env.local"), "SHOPWARE_DEPLOY_ENV=staging\n").unwrap();
        let env = ShopEnv::load(shop.clone(), &HashMap::new()).unwrap();
        assert_eq!(env.get("SHOPWARE_SHOP_ID"), Some("acme"));
        assert_eq!(env.get("SHOPWARE_DEPLOY_ENV"), Some("staging"));
        let _ = fs::remove_dir_all(&shop);
    }

    #[test]
    fn empty_shared_deploy_env_does_not_hide_host_file() {
        let shop = temp_shop("deploy-env-empty-shared");
        fs::write(
            shop.join(".env"),
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=\n",
        )
        .unwrap();
        fs::write(shop.join(".env.local"), "SHOPWARE_DEPLOY_ENV=dev\n").unwrap();
        let env = ShopEnv::load(shop.clone(), &HashMap::new()).unwrap();
        assert_eq!(env.get("SHOPWARE_DEPLOY_ENV"), Some("dev"));
        let _ = fs::remove_dir_all(&shop);
    }

    #[test]
    fn empty_shared_compose_project_name_does_not_hide_host_file() {
        let shop = temp_shop("cpn-empty-shared");
        fs::write(
            shop.join(".env"),
            "SHOPWARE_SHOP_ID=acme\nCOMPOSE_PROJECT_NAME=\n",
        )
        .unwrap();
        fs::write(
            shop.join(".env.local"),
            "SHOPWARE_DEPLOY_ENV=dev\nCOMPOSE_PROJECT_NAME=acme-dev\n",
        )
        .unwrap();
        let env = ShopEnv::load(shop.clone(), &HashMap::new()).unwrap();
        assert_eq!(env.get("COMPOSE_PROJECT_NAME"), Some("acme-dev"));
        let _ = fs::remove_dir_all(&shop);
    }

    #[test]
    fn compose_project_name_from_local_wins_over_shared_leftover() {
        let shop = temp_shop("cpn-local-wins");
        fs::write(
            shop.join(".env"),
            "SHOPWARE_SHOP_ID=acme\nCOMPOSE_PROJECT_NAME=sw-shop-acme\n",
        )
        .unwrap();
        fs::write(
            shop.join(".env.local"),
            "SHOPWARE_DEPLOY_ENV=dev\nCOMPOSE_PROJECT_NAME=acme-dev\n",
        )
        .unwrap();
        let env = ShopEnv::load(shop.clone(), &HashMap::new()).unwrap();
        assert_eq!(env.get("COMPOSE_PROJECT_NAME"), Some("acme-dev"));
        assert_eq!(vps_project_name_opt(&env).as_deref(), Some("acme-dev"));
        let _ = fs::remove_dir_all(&shop);
    }

    #[test]
    fn empty_local_deploy_env_does_not_wipe_shared_leftover() {
        let shop = temp_shop("deploy-env-empty-local");
        fs::write(
            shop.join(".env"),
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=live\n",
        )
        .unwrap();
        fs::write(shop.join(".env.local"), "SHOPWARE_DEPLOY_ENV=\n").unwrap();
        let env = ShopEnv::load(shop.clone(), &HashMap::new()).unwrap();
        assert_eq!(env.get("SHOPWARE_DEPLOY_ENV"), Some("live"));
        let _ = fs::remove_dir_all(&shop);
    }

    #[test]
    fn deploy_env_from_prod_wins_over_local() {
        let shop = temp_shop("deploy-env-prod-wins");
        fs::write(shop.join(".env"), "SHOPWARE_SHOP_ID=acme\n").unwrap();
        fs::write(shop.join(".env.local"), "SHOPWARE_DEPLOY_ENV=dev\n").unwrap();
        fs::write(shop.join(".env.prod"), "SHOPWARE_DEPLOY_ENV=live\n").unwrap();
        let env = ShopEnv::load(shop.clone(), &HashMap::new()).unwrap();
        assert_eq!(env.get("SHOPWARE_DEPLOY_ENV"), Some("live"));
        let _ = fs::remove_dir_all(&shop);
    }

    #[test]
    fn local_from_aliases() {
        assert!(is_local_source(None));
        assert!(is_local_source(Some("local")));
        assert!(is_local_source(Some("LOCAL")));
        assert!(is_local_source(Some("this")));
        assert!(!is_local_source(Some("live")));
    }

    #[test]
    fn local_data_root_override_then_derived() {
        let mut vars = HashMap::new();
        vars.insert("SHOPWARE_SHOP_ID".into(), "acme".into());
        vars.insert("SHOPWARE_DEPLOY_ENV".into(), "staging".into());
        vars.insert("SHOPWARE_DATA_ROOT".into(), "/data/shopware".into());
        let env = ShopEnv::from_vars(PathBuf::from("/shop"), vars);
        assert_eq!(
            derive_local_data_root(&env).unwrap(),
            PathBuf::from("/data/shopware")
        );
        assert_eq!(
            resolve_data_root(&env).unwrap(),
            PathBuf::from("/data/shopware")
        );

        let mut vars = HashMap::new();
        vars.insert("SHOPWARE_SHOP_ID".into(), "acme".into());
        vars.insert("SHOPWARE_DEPLOY_ENV".into(), "live".into());
        let env = ShopEnv::from_vars(PathBuf::from("/shop"), vars);
        assert_eq!(
            derive_local_data_root(&env).unwrap(),
            PathBuf::from("/var/lib/shopware/data/acme/live")
        );
    }

    #[test]
    fn remote_data_root_override_else_live_formula() {
        let mut vars = HashMap::new();
        vars.insert("SHOPWARE_SHOP_ID".into(), "acme".into());
        vars.insert("SHOPWARE_DATA_BASE".into(), "/opt/data".into());
        let env = ShopEnv::from_vars(PathBuf::from("/shop"), vars);
        assert_eq!(
            remote_data_root(&env, "acme"),
            PathBuf::from("/opt/data/acme/live")
        );

        let mut vars = HashMap::new();
        vars.insert("SHOPWARE_REMOTE_DATA_ROOT".into(), "/mnt/uploads".into());
        let env = ShopEnv::from_vars(PathBuf::from("/shop"), vars);
        assert_eq!(
            remote_data_root(&env, "acme"),
            PathBuf::from("/mnt/uploads")
        );
    }

    #[test]
    fn data_root_custom_base() {
        let mut vars = HashMap::new();
        vars.insert("SHOPWARE_SHOP_ID".into(), "acme".into());
        vars.insert("SHOPWARE_DEPLOY_ENV".into(), "dev".into());
        vars.insert("SHOPWARE_DATA_BASE".into(), "/opt/data".into());
        let env = ShopEnv::from_vars(PathBuf::from("/shop"), vars);
        assert_eq!(
            resolve_data_root(&env).unwrap(),
            PathBuf::from("/opt/data/acme/dev")
        );
    }

    #[test]
    fn snapshot_dir_flag_relative_and_default() {
        let mut vars = HashMap::new();
        vars.insert("SHOPWARE_SHOP_ID".into(), "acme".into());
        let env = ShopEnv::from_vars(PathBuf::from("/shop"), vars);
        assert_eq!(
            resolve_snapshot_dir(None, &env, Path::new("/shop")),
            PathBuf::from("/shop/var/runtime-sync")
        );
        assert_eq!(
            resolve_snapshot_dir(Some("tmp/s"), &env, Path::new("/shop")),
            PathBuf::from("/shop/tmp/s")
        );
        assert_eq!(
            resolve_snapshot_dir(Some("/abs/s"), &env, Path::new("/shop")),
            PathBuf::from("/abs/s")
        );
    }

    #[test]
    fn env_truthy_matches_overlay() {
        assert!(env_truthy(Some("1")));
        assert!(env_truthy(Some("true")));
        assert!(env_truthy(Some("YES")));
        assert!(env_truthy(Some("On")));
        assert!(!env_truthy(Some("0")));
        assert!(!env_truthy(Some("false")));
        assert!(!env_truthy(Some("")));
        assert!(!env_truthy(None));
        let mut vars = HashMap::new();
        vars.insert("SHOPWARE_ALLOW_LIVE_RESTORE".into(), "yes".into());
        let env = ShopEnv::from_vars(PathBuf::from("/shop"), vars);
        assert!(allow_live_restore(&env));
    }

    #[test]
    fn init_compose_dir_allows_example_without_env() {
        let shop = temp_shop("init-example-only");
        fs::write(shop.join(".env.example"), "SHOPWARE_SHOP_ID=\n").unwrap();
        let mut process = HashMap::new();
        process.insert("COMPOSE_DIR".into(), shop.to_string_lossy().into_owned());
        let got = resolve_compose_dir_init(&process, Path::new("/tmp")).unwrap();
        assert_eq!(got, fs::canonicalize(&shop).unwrap());
        let walked = resolve_compose_dir_init(&HashMap::new(), &shop).unwrap();
        assert_eq!(walked, fs::canonicalize(&shop).unwrap());
        let _ = fs::remove_dir_all(&shop);
    }

    #[test]
    fn init_compose_dir_missing_directory() {
        let mut process = HashMap::new();
        process.insert("COMPOSE_DIR".into(), "/no/such/fyrst-cli-shop".into());
        let err = resolve_compose_dir_init(&process, Path::new("/tmp")).unwrap_err();
        assert!(
            err.to_string().contains("Cannot cd to COMPOSE_DIR="),
            "{err}"
        );
    }

    #[test]
    fn backup_keys_from_env_then_process_wins() {
        let shop = temp_shop("backup-env");
        fs::write(
            shop.join(".env"),
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=live\nBACKUP_TARGET=/from-file\nBACKUP_KEEP_DAYS=7\n",
        )
        .unwrap();
        let mut process = HashMap::new();
        process.insert("BACKUP_TARGET".into(), "/from-proc".into());
        let env = ShopEnv::load(shop.clone(), &process).unwrap();
        assert_eq!(env.get("BACKUP_TARGET"), Some("/from-proc"));
        assert_eq!(env.get("BACKUP_KEEP_DAYS"), Some("7"));
        let _ = fs::remove_dir_all(&shop);
    }
}
