//! Shop-root discovery, `.env` loading, and Compose project name.
//!
//! Identity overlay matches `deploy/lib/sync-commands.sh`: files are loaded
//! then process-env presets win for shop identity keys (non-empty only).

use super::envfile::parse_env_file;
use super::error::Error;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const IDENTITY_KEYS: &[&str] = &[
    "SYNC_ENV",
    "IMAGE",
    "IMAGE_TAG",
    "SYNC_DATA_ROOT",
    "SHOPWARE_SHOP_ID",
    "SHOPWARE_DEPLOY_ENV",
    "COMPOSE_PROJECT_NAME",
    "SHOPWARE_DATA_ROOT",
    "SHOPWARE_DATA_BASE",
    "SYNC_SOURCE_ENV",
    "SYNC_REMOTE_DATA_ROOT",
];

/// Process-env wins (non-empty) after `deploy/backup.env`, matching
/// `backup-runtime.sh` PRESET_BACKUP_* restoration.
pub const BACKUP_PRESET_KEYS: &[&str] = &[
    "BACKUP_TARGET",
    "BACKUP_KEEP_DAYS",
    "BACKUP_SSH_KEY",
    "BACKUP_SSH_PORT",
    "BACKUP_DB_DUMP",
];

const ENV_FILES: &[&str] = &[".env", ".env.prod", "deploy/sync.env"];

/// Overlay `sync-runtime-local.sh` sources shop-root `.env` then `deploy/sync.env`
/// (not `.env.prod`).
pub const SYNC_LOCAL_ENV_FILES: &[&str] = &[".env", "deploy/sync.env"];

/// VPS release/rollback load `.env` then `.env.prod` only (no `deploy/sync.env`).
const VPS_ENV_FILES: &[&str] = &[".env", ".env.prod"];

/// CI / process-env keys that win over `.env` after the file load (non-empty).
const VPS_PROCESS_WINS: &[&str] = &[
    "SYNC_ENV",
    "IMAGE",
    "IMAGE_TAG",
    "SYNC_DATA_ROOT",
    "SHOPWARE_SHOP_ID",
    "SHOPWARE_DEPLOY_ENV",
    "COMPOSE_PROJECT_NAME",
    "SHOPWARE_DATA_ROOT",
    "SHOPWARE_DATA_BASE",
    "SYNC_SOURCE_ENV",
    "SYNC_REMOTE_DATA_ROOT",
    "SMOKE_URL",
    "COMPOSE_PROFILES",
    "ROLLBACK_ON_SMOKE_FAIL",
    "SKIP_PULL",
    "PULL_POLICY",
];

pub const COMPOSE_FILES: &[&str] = &[
    "deploy/compose.yaml",
    "deploy/compose.prod.yaml",
    "deploy/compose.vps.yaml",
];

/// Overlay `DEFAULT_DATA_BASE` (`deploy/lib/identity.sh`).
pub const DEFAULT_DATA_BASE: &str = "/var/lib/shopware/data";

#[derive(Debug)]
pub struct ShopEnv {
    pub compose_dir: PathBuf,
    vars: HashMap<String, String>,
}

impl ShopEnv {
    pub fn load(compose_dir: PathBuf, process: &HashMap<String, String>) -> Result<Self, Error> {
        Self::load_files(compose_dir, process, ENV_FILES, IDENTITY_KEYS)
    }

    /// Overlay `vps_load_shop_env` + `vps_restore_cli_env`: `.env` / `.env.prod`,
    /// then non-empty process `IMAGE` / `IMAGE_TAG` (and other VPS knobs) win.
    pub fn load_vps(
        compose_dir: PathBuf,
        process: &HashMap<String, String>,
    ) -> Result<Self, Error> {
        Self::load_files(compose_dir, process, VPS_ENV_FILES, VPS_PROCESS_WINS)
    }

    /// Overlay `sync-runtime-local.sh`: `.env` then `deploy/sync.env` (no `.env.prod`).
    pub fn load_sync_local(
        compose_dir: PathBuf,
        process: &HashMap<String, String>,
    ) -> Result<Self, Error> {
        Self::load_files(compose_dir, process, SYNC_LOCAL_ENV_FILES, IDENTITY_KEYS)
    }

    /// `.env` / `.env.prod` / `deploy/sync.env` then `deploy/backup.env` with BACKUP_* presets.
    pub fn load_backup(
        compose_dir: PathBuf,
        process: &HashMap<String, String>,
    ) -> Result<Self, Error> {
        let mut env = Self::load(compose_dir, process)?;
        env.load_backup_overlay(process)?;
        Ok(env)
    }

    fn load_files(
        compose_dir: PathBuf,
        process: &HashMap<String, String>,
        files: &[&str],
        process_wins: &[&str],
    ) -> Result<Self, Error> {
        let mut vars = process.clone();
        for rel in files {
            let path = compose_dir.join(rel);
            if path.is_file() {
                let contents = fs::read_to_string(&path)
                    .map_err(|e| Error::fail(format!("cannot read {}: {e}", path.display())))?;
                for (k, v) in parse_env_file(&contents) {
                    vars.insert(k, v);
                }
            }
        }
        for key in process_wins {
            if let Some(preset) = process.get(*key).filter(|s| !s.is_empty()) {
                vars.insert((*key).to_string(), preset.clone());
            }
        }
        Ok(Self { compose_dir, vars })
    }

    /// Non-empty value, same idea as bash `${VAR:-}` for required checks.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars
            .get(key)
            .map(String::as_str)
            .filter(|s| !s.is_empty())
    }

    pub fn merge_env_file(&mut self, path: &Path) -> Result<(), Error> {
        if !path.is_file() {
            return Ok(());
        }
        let contents = fs::read_to_string(path)
            .map_err(|e| Error::fail(format!("cannot read {}: {e}", path.display())))?;
        for (k, v) in parse_env_file(&contents) {
            self.vars.insert(k, v);
        }
        Ok(())
    }

    pub fn restore_process_presets(&mut self, process: &HashMap<String, String>, keys: &[&str]) {
        for key in keys {
            if let Some(preset) = process.get(*key).filter(|s| !s.is_empty()) {
                self.vars.insert((*key).to_string(), preset.clone());
            }
        }
    }

    /// Overlay `deploy/backup.env` then process-env BACKUP_* / identity presets.
    pub fn load_backup_overlay(&mut self, process: &HashMap<String, String>) -> Result<(), Error> {
        self.merge_env_file(&self.compose_dir.join("deploy/backup.env"))?;
        self.restore_process_presets(process, IDENTITY_KEYS);
        self.restore_process_presets(process, BACKUP_PRESET_KEYS);
        Ok(())
    }

    #[cfg(test)]
    pub fn from_vars(compose_dir: PathBuf, vars: HashMap<String, String>) -> Self {
        Self { compose_dir, vars }
    }
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

/// `init-env` may run before `.env` exists (copy from `.env.example`).
pub fn looks_like_shop_root_init(dir: &Path) -> bool {
    (dir.join(".env").is_file() || dir.join(".env.example").is_file()) && has_deploy_marker(dir)
}

/// Shop checkout for `init-env`. Unlike [`resolve_compose_dir`], `.env` may still
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
                "SHOPWARE_DEPLOY_ENV is required (live|staging|playground|dev). Set it in shop-root .env.",
            )
        })
}

pub const DEFAULT_ARCHIVE_IMAGE: &str = "alpine:3.20";

/// `(project_name, derived_from_shop_id_and_env)`.
pub fn derive_project_name(env: &ShopEnv) -> Result<(String, bool), Error> {
    if let Some(n) = env.get("COMPOSE_PROJECT_NAME") {
        return Ok((n.to_string(), false));
    }
    match (env.get("SHOPWARE_SHOP_ID"), env.get("SHOPWARE_DEPLOY_ENV")) {
        (Some(id), Some(deploy_env)) => Ok((format!("{id}-{deploy_env}"), true)),
        _ => Err(Error::fail(
            "Set COMPOSE_PROJECT_NAME in .env (must be unique on this Docker host), or set SHOPWARE_SHOP_ID and SHOPWARE_DEPLOY_ENV to derive ${SHOPWARE_SHOP_ID}-${SHOPWARE_DEPLOY_ENV}.",
        )),
    }
}

/// Work directory for sync snapshot/restore (`--snapshot-dir` or `SYNC_SNAPSHOT_DIR`).
pub fn resolve_snapshot_dir(cli_dir: Option<&str>, env: &ShopEnv, compose_dir: &Path) -> PathBuf {
    if let Some(d) = cli_dir.map(str::trim).filter(|s| !s.is_empty()) {
        let p = PathBuf::from(d);
        if p.is_absolute() {
            p
        } else {
            compose_dir.join(p)
        }
    } else if let Some(d) = env.get("SYNC_SNAPSHOT_DIR") {
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

pub fn archive_image(env: &ShopEnv) -> String {
    env.get("SYNC_ARCHIVE_IMAGE")
        .unwrap_or(DEFAULT_ARCHIVE_IMAGE)
        .to_string()
}

pub fn derived_data_root(env: &ShopEnv, shop_id: &str, deploy_env: &str) -> PathBuf {
    PathBuf::from(data_base(env)).join(shop_id).join(deploy_env)
}

/// Bind-mount root on this host (`SYNC_DATA_ROOT` / `SHOPWARE_DATA_ROOT` / derived).
pub fn derive_local_data_root(env: &ShopEnv) -> Result<PathBuf, Error> {
    Ok(local_data_root(env)?.0)
}

/// Local bind-mount root plus whether the path was derived (overlay logs that case).
pub fn local_data_root(env: &ShopEnv) -> Result<(PathBuf, bool), Error> {
    if let Some(p) = env.get("SYNC_DATA_ROOT") {
        return Ok((PathBuf::from(p), false));
    }
    if let Some(p) = env.get("SHOPWARE_DATA_ROOT") {
        return Ok((PathBuf::from(p), false));
    }
    let shop_id = require_shop_id(env)?;
    let deploy_env = env.get("SHOPWARE_DEPLOY_ENV").ok_or_else(|| {
        Error::fail(
            "SHOPWARE_DEPLOY_ENV is required to derive SHOPWARE_DATA_ROOT (live|staging|playground|dev). Set it in .env, or set SHOPWARE_DATA_ROOT / SYNC_DATA_ROOT explicitly.",
        )
    })?;
    Ok((derived_data_root(env, &shop_id, deploy_env), true))
}

/// Alias used by `sync restore` (same resolution as snapshot).
pub fn resolve_data_root(env: &ShopEnv) -> Result<PathBuf, Error> {
    derive_local_data_root(env)
}

/// Backup bind-mount root: `SHOPWARE_DATA_ROOT` or `$SHOPWARE_DATA_BASE/$shop_id/$deploy_env`.
pub fn resolve_backup_data_root(env: &ShopEnv, shop_id: &str, deploy_env: &str) -> (PathBuf, bool) {
    if let Some(root) = env.get("SHOPWARE_DATA_ROOT") {
        return (PathBuf::from(root), false);
    }
    (derived_data_root(env, shop_id, deploy_env), true)
}

/// Remote env directory (`SYNC_SOURCE_ENV`, else `--from` alias, else `live`).
pub fn source_env_for_remote(from: &str, env: &ShopEnv) -> String {
    if let Some(s) = env.get("SYNC_SOURCE_ENV") {
        return s.to_string();
    }
    let from_lc = from.trim().to_ascii_lowercase();
    if !from_lc.is_empty() && from_lc != "local" && from_lc != "this" {
        from_lc
    } else {
        "live".into()
    }
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
    fn vps_ci_image_tag_wins_over_env_latest() {
        let shop = temp_shop("vps-ci-tag");
        fs::write(
            shop.join(".env"),
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=live
IMAGE=ghcr.io/from-file/shop
IMAGE_TAG=latest
SMOKE_URL=http://from-file
COMPOSE_PROFILES=redis
",
        )
        .unwrap();
        fs::write(
            shop.join("deploy/sync.env"),
            "IMAGE_TAG=from-sync-env\nSMOKE_URL=http://from-sync\n",
        )
        .unwrap();
        let mut process = HashMap::new();
        process.insert("IMAGE".into(), "ghcr.io/from-ci/shop".into());
        process.insert("IMAGE_TAG".into(), "abc123deadbeef".into());
        process.insert("SMOKE_URL".into(), "http://127.0.0.1:8000".into());
        let env = ShopEnv::load_vps(shop.clone(), &process).unwrap();
        assert_eq!(env.get("IMAGE"), Some("ghcr.io/from-ci/shop"));
        assert_eq!(env.get("IMAGE_TAG"), Some("abc123deadbeef"));
        assert_eq!(env.get("SMOKE_URL"), Some("http://127.0.0.1:8000"));
        assert_eq!(env.get("COMPOSE_PROFILES"), Some("redis"));
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
    fn compose_project_name_wins() {
        let mut vars = HashMap::new();
        vars.insert("COMPOSE_PROJECT_NAME".into(), "sw-shop-acme".into());
        vars.insert("SHOPWARE_SHOP_ID".into(), "acme".into());
        vars.insert("SHOPWARE_DEPLOY_ENV".into(), "live".into());
        let env = ShopEnv::from_vars(PathBuf::from("/tmp/x"), vars);
        assert_eq!(
            derive_project_name(&env).unwrap(),
            ("sw-shop-acme".into(), false)
        );
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
    fn local_data_root_prefers_sync_then_shopware_then_derived() {
        let mut vars = HashMap::new();
        vars.insert("SHOPWARE_SHOP_ID".into(), "acme".into());
        vars.insert("SHOPWARE_DEPLOY_ENV".into(), "staging".into());
        vars.insert("SHOPWARE_DATA_ROOT".into(), "/data/shopware".into());
        vars.insert("SYNC_DATA_ROOT".into(), "/override".into());
        let env = ShopEnv::from_vars(PathBuf::from("/shop"), vars);
        assert_eq!(
            derive_local_data_root(&env).unwrap(),
            PathBuf::from("/override")
        );
        assert_eq!(resolve_data_root(&env).unwrap(), PathBuf::from("/override"));

        let mut vars = HashMap::new();
        vars.insert("SHOPWARE_SHOP_ID".into(), "acme".into());
        vars.insert("SHOPWARE_DEPLOY_ENV".into(), "staging".into());
        vars.insert("SHOPWARE_DATA_ROOT".into(), "/data/shopware".into());
        let env = ShopEnv::from_vars(PathBuf::from("/shop"), vars);
        assert_eq!(
            derive_local_data_root(&env).unwrap(),
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
    fn remote_source_env_from_from_alias() {
        let env = ShopEnv::from_vars(PathBuf::from("/shop"), HashMap::new());
        assert_eq!(source_env_for_remote("live", &env), "live");
        assert_eq!(source_env_for_remote("staging", &env), "staging");
        let mut vars = HashMap::new();
        vars.insert("SYNC_SOURCE_ENV".into(), "playground".into());
        let env = ShopEnv::from_vars(PathBuf::from("/shop"), vars);
        assert_eq!(source_env_for_remote("live", &env), "playground");
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
    fn backup_env_file_then_process_wins() {
        let shop = temp_shop("backup-env");
        fs::write(
            shop.join(".env"),
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=live\n",
        )
        .unwrap();
        fs::create_dir_all(shop.join("deploy")).unwrap();
        fs::write(
            shop.join("deploy/backup.env"),
            "BACKUP_TARGET=/from-file\nBACKUP_KEEP_DAYS=7\n",
        )
        .unwrap();
        let mut process = HashMap::new();
        process.insert("BACKUP_TARGET".into(), "/from-proc".into());
        let mut env = ShopEnv::load(shop.clone(), &process).unwrap();
        env.load_backup_overlay(&process).unwrap();
        assert_eq!(env.get("BACKUP_TARGET"), Some("/from-proc"));
        assert_eq!(env.get("BACKUP_KEEP_DAYS"), Some("7"));
        let _ = fs::remove_dir_all(&shop);
    }
}
