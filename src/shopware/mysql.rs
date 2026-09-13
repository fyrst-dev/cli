//! MySQL/MariaDB connection helpers and Compose `mysql` bring-up.
//!
//! Used by `shopware db import` (and `sync restore` for the DB). Passwords
//! must never appear in log lines.

use super::env::ShopEnv;
use super::envfile::urldecode;
use super::error::Error;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const DEFAULT_MYSQL_CLIENT_IMAGE: &str = "mysql:8.4";
pub const DEFAULT_MARIADB_CLIENT_IMAGE: &str = "mariadb:11.4";

/// POSIX sh: runs inside the bundled mysql/mariadb container (official images
/// use dash). Matches recipes `mysql_restore_sh`.
pub const MYSQL_RESTORE_SH: &str = r#"set -eu
DB="${MYSQL_DATABASE:-shopware}"
if command -v mariadb >/dev/null 2>&1; then
  CLI=mariadb
elif command -v mysql >/dev/null 2>&1; then
  CLI=mysql
else
  echo "Neither mysql nor mariadb client is in the mysql container" >&2
  exit 1
fi
if "$CLI" -uroot --protocol=socket -e "SELECT 1" >/dev/null 2>&1; then
  "$CLI" -uroot --protocol=socket --max-allowed-packet=1G "$DB"
elif [ -n "${MYSQL_ROOT_PASSWORD:-}" ] && "$CLI" -uroot -p"${MYSQL_ROOT_PASSWORD}" -h127.0.0.1 -e "SELECT 1" >/dev/null 2>&1; then
  "$CLI" -uroot -p"${MYSQL_ROOT_PASSWORD}" -h127.0.0.1 --max-allowed-packet=1G "$DB"
elif [ -n "${MYSQL_USER:-}" ] && [ -n "${MYSQL_PASSWORD:-}" ] && "$CLI" -u"${MYSQL_USER}" -p"${MYSQL_PASSWORD}" -h127.0.0.1 -e "SELECT 1" >/dev/null 2>&1; then
  "$CLI" -u"${MYSQL_USER}" -p"${MYSQL_PASSWORD}" -h127.0.0.1 --max-allowed-packet=1G "$DB"
else
  "$CLI" -uroot --protocol=socket --max-allowed-packet=1G "$DB"
fi
"#;

pub const MYSQL_WAIT_SH: &str = r#"set -eu
i=0
while [ "$i" -lt 40 ]; do
  i=$((i + 1))
  if command -v mysqladmin >/dev/null 2>&1 && mysqladmin ping -h 127.0.0.1 --silent; then
    exit 0
  fi
  if command -v mariadb-admin >/dev/null 2>&1 && mariadb-admin ping -h 127.0.0.1 --silent; then
    exit 0
  fi
  sleep 2
done
echo "mysql did not become ready" >&2
exit 1
"#;

/// One-shot client container stdin restore (recipes `restore_db_via_url`).
pub const EXTERNAL_RESTORE_SH: &str = r#"set -eu
if command -v mariadb >/dev/null; then C=mariadb; else C=mysql; fi
"$C" -h"$DUMP_HOST" -P"$DUMP_PORT" -u"$DUMP_USER" --max-allowed-packet=1G "$DUMP_DB"
"#;

#[derive(Clone, PartialEq, Eq)]
pub struct DatabaseUrl {
    pub scheme: String,
    pub user: String,
    pub password: String,
    pub host: String,
    pub port: String,
    pub name: String,
}

impl fmt::Debug for DatabaseUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatabaseUrl")
            .field("scheme", &self.scheme)
            .field("user", &self.user)
            .field(
                "password",
                &if self.password.is_empty() { "" } else { "***" },
            )
            .field("host", &self.host)
            .field("port", &self.port)
            .field("name", &self.name)
            .finish()
    }
}

pub fn parse_database_url(url: &str) -> Option<DatabaseUrl> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let (scheme, rest) = url.split_once("://")?;
    let rest = rest.split_once('?').map(|(a, _)| a).unwrap_or(rest);
    let (creds, hostpart) = match rest.split_once('@') {
        Some((c, h)) => (Some(c), h),
        None => (None, rest),
    };
    let mut user = String::new();
    let mut password = String::new();
    if let Some(creds) = creds {
        if let Some((u, p)) = creds.split_once(':') {
            user = urldecode(u);
            password = urldecode(p);
        } else {
            user = urldecode(creds);
        }
    }
    let (hp, name_part) = match hostpart.split_once('/') {
        Some((h, n)) => (h, n),
        None => (hostpart, ""),
    };
    let name = name_part.split('/').next().unwrap_or("").to_string();
    let (host, port) = parse_host_port(hp);
    Some(DatabaseUrl {
        scheme: scheme.to_string(),
        user,
        password,
        host,
        port,
        name,
    })
}

fn parse_host_port(hp: &str) -> (String, String) {
    if let Some(rest) = hp.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            let host = rest[..end].to_string();
            let after = &rest[end + 1..];
            let port = after
                .strip_prefix(':')
                .filter(|s| !s.is_empty())
                .unwrap_or("3306");
            return (host, port.to_string());
        }
    }
    if hp.matches(':').count() == 1 {
        if let Some((h, p)) = hp.split_once(':') {
            let port = if p.is_empty() {
                "3306".to_string()
            } else {
                p.to_string()
            };
            return (h.to_string(), port);
        }
    }
    (hp.to_string(), "3306".into())
}

pub fn client_image_for_url(scheme: &str, env: &ShopEnv) -> String {
    if let Some(img) = env.get("SYNC_MYSQL_CLIENT_IMAGE") {
        return img.to_string();
    }
    if scheme.eq_ignore_ascii_case("mariadb") {
        DEFAULT_MARIADB_CLIENT_IMAGE.to_string()
    } else {
        DEFAULT_MYSQL_CLIENT_IMAGE.to_string()
    }
}

pub fn compose_argv(files: &[String]) -> Vec<String> {
    let mut a = vec!["compose".into(), "--env-file".into(), ".env".into()];
    for f in files {
        a.push("-f".into());
        a.push(f.clone());
    }
    a
}

pub fn compose_cli_log(files: &[String]) -> String {
    let mut s = String::from("docker compose --env-file .env");
    for f in files {
        s.push_str(" -f ");
        s.push_str(f);
    }
    s
}

pub fn require_docker() -> Result<(), Error> {
    match Command::new("docker")
        .arg("info")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(s) if s.success() => Ok(()),
        Ok(_) => Err(Error::fail(
            "Cannot talk to the Docker daemon. Add this user to the docker group or run where the daemon is reachable.",
        )),
        Err(_) => Err(Error::fail(
            "Missing command 'docker'. Install Docker Engine + Compose v2.",
        )),
    }
}

pub fn require_gzip() -> Result<(), Error> {
    match Command::new("gzip")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(s) if s.success() => Ok(()),
        _ => Err(Error::fail(
            "Missing command 'gzip' (needed to decompress .sql.gz dumps).",
        )),
    }
}

pub fn compose_up_mysql(compose_dir: &Path, files: &[String]) -> Result<(), Error> {
    if files.is_empty() {
        return Err(Error::fail(
            "No compose files found under shop root; cannot start service mysql.",
        ));
    }
    let mut args = compose_argv(files);
    args.extend([
        "up".into(),
        "-d".into(),
        "--no-build".into(),
        "mysql".into(),
    ]);
    let status = Command::new("docker")
        .args(&args)
        .current_dir(compose_dir)
        .status()
        .map_err(|e| Error::fail(format!("could not exec docker compose: {e}")))?;
    if !status.success() {
        return Err(Error::fail(
            "docker compose up mysql failed. Is Docker running, and is the mysql service defined?",
        ));
    }
    wait_mysql(compose_dir, files)
}

fn wait_mysql(compose_dir: &Path, files: &[String]) -> Result<(), Error> {
    let mut args = compose_argv(files);
    args.extend([
        "exec".into(),
        "-T".into(),
        "mysql".into(),
        "sh".into(),
        "-c".into(),
        MYSQL_WAIT_SH.to_string(),
    ]);
    let status = Command::new("docker")
        .args(&args)
        .current_dir(compose_dir)
        .status()
        .map_err(|e| Error::fail(format!("could not exec docker compose wait: {e}")))?;
    if !status.success() {
        return Err(Error::fail(
            "mysql did not become ready via compose exec. Start the stack or check the mysql service.",
        ));
    }
    Ok(())
}

pub fn verify_gzip_magic(path: &Path) -> Result<(), Error> {
    let mut mag = [0u8; 2];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut mag))
        .map_err(|e| Error::fail(format!("cannot read {}: {e}", path.display())))?;
    if mag != [0x1f, 0x8b] {
        return Err(Error::fail(format!(
            "Dump at {} is not valid gzip (expected a .sql.gz file).",
            path.display()
        )));
    }
    Ok(())
}

pub fn collect_env_secrets(env: &ShopEnv, url: Option<&DatabaseUrl>) -> Vec<String> {
    let mut secrets = Vec::new();
    for key in ["MYSQL_PASSWORD", "MYSQL_ROOT_PASSWORD"] {
        if let Some(v) = env.get(key) {
            if !v.is_empty() {
                secrets.push(v.to_string());
            }
        }
    }
    if let Some(u) = url {
        if !u.password.is_empty() {
            secrets.push(u.password.clone());
        }
        if let Some(raw) = env.get("DATABASE_URL") {
            if raw.contains(':') {
                secrets.push(raw.to_string());
            }
        }
    }
    secrets
}

pub fn log_contains_secret(text: &str, secrets: &[String]) -> bool {
    secrets
        .iter()
        .any(|s| !s.is_empty() && text.contains(s.as_str()))
}

pub fn find_snapshot_dump(dir: &Path) -> Result<PathBuf, Error> {
    let gz = dir.join("db.sql.gz");
    let sql = dir.join("db.sql");
    if gz.is_file() {
        Ok(gz)
    } else if sql.is_file() {
        Ok(sql)
    } else {
        Err(Error::fail(format!(
            "No db.sql.gz (or db.sql) in {}",
            dir.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;

    fn env_from(pairs: &[(&str, &str)]) -> ShopEnv {
        let mut vars = HashMap::new();
        for (k, v) in pairs {
            vars.insert((*k).to_string(), (*v).to_string());
        }
        ShopEnv::from_vars(PathBuf::from("/shop"), vars)
    }

    #[test]
    fn parse_url_percent_password() {
        let u = parse_database_url("mysql://user:p%40ss@mysql:3306/shopware").unwrap();
        assert_eq!(u.user, "user");
        assert_eq!(u.password, "p@ss");
        assert_eq!(u.host, "mysql");
        assert_eq!(u.port, "3306");
        assert_eq!(u.name, "shopware");
    }

    #[test]
    fn parse_url_ipv6() {
        let u = parse_database_url("mysql://u:p@[::1]:3307/db").unwrap();
        assert_eq!(u.host, "::1");
        assert_eq!(u.port, "3307");
        assert_eq!(u.name, "db");
    }

    #[test]
    fn client_image_scheme_and_override() {
        let env = env_from(&[]);
        assert_eq!(
            client_image_for_url("mysql", &env),
            DEFAULT_MYSQL_CLIENT_IMAGE
        );
        assert_eq!(
            client_image_for_url("mariadb", &env),
            DEFAULT_MARIADB_CLIENT_IMAGE
        );
        let env = env_from(&[("SYNC_MYSQL_CLIENT_IMAGE", "mysql:8.0")]);
        assert_eq!(client_image_for_url("mariadb", &env), "mysql:8.0");
    }

    #[test]
    fn secrets_never_include_empty() {
        let env = env_from(&[("MYSQL_PASSWORD", "s3cret-value")]);
        let s = collect_env_secrets(&env, None);
        assert_eq!(s, vec!["s3cret-value"]);
        assert!(log_contains_secret("oops s3cret-value here", &s));
        assert!(!log_contains_secret("docker compose exec -T mysql", &s));
    }

    #[test]
    fn find_snapshot_prefers_gz() {
        let dir = std::env::temp_dir().join(format!(
            "fyrst-cli-dump-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("db.sql"), b"sql").unwrap();
        fs::write(dir.join("db.sql.gz"), b"gz").unwrap();
        assert!(find_snapshot_dump(&dir).unwrap().ends_with("db.sql.gz"));
        fs::remove_file(dir.join("db.sql.gz")).unwrap();
        assert!(find_snapshot_dump(&dir).unwrap().ends_with("db.sql"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn gzip_magic() {
        let dir = std::env::temp_dir().join(format!(
            "fyrst-cli-magic-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("db.sql.gz");
        fs::write(&p, b"not gzip").unwrap();
        assert!(verify_gzip_magic(&p)
            .unwrap_err()
            .to_string()
            .contains("gzip"));
        fs::write(&p, [0x1f, 0x8b, 0x08, 0x00]).unwrap();
        verify_gzip_magic(&p).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
}
