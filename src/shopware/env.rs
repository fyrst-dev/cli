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

const ENV_FILES: &[&str] = &[".env", ".env.prod", "deploy/sync.env"];

pub const COMPOSE_FILES: &[&str] = &[
    "deploy/compose.yaml",
    "deploy/compose.prod.yaml",
    "deploy/compose.vps.yaml",
];

pub struct ShopEnv {
    pub compose_dir: PathBuf,
    vars: HashMap<String, String>,
}

impl ShopEnv {
    pub fn load(compose_dir: PathBuf, process: &HashMap<String, String>) -> Result<Self, Error> {
        let mut vars = process.clone();
        for rel in ENV_FILES {
            let path = compose_dir.join(rel);
            if path.is_file() {
                let contents = fs::read_to_string(&path)
                    .map_err(|e| Error::fail(format!("cannot read {}: {e}", path.display())))?;
                for (k, v) in parse_env_file(&contents) {
                    vars.insert(k, v);
                }
            }
        }
        for key in IDENTITY_KEYS {
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

/// `(project_name, derived_from_shop_id_and_env)`.
#[allow(dead_code)]
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
}
