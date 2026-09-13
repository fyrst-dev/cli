//! SSH source resolution for remote `--from` (recipes `deploy/lib/sync-ssh.sh`).
//!
//! Used by `shopware sync snapshot` to rsync bind-mount trees. Never used to
//! run `shopware-cli project dump` on the remote.

use super::env::{derived_data_root, require_cmd, source_env_for_remote, ShopEnv};
use super::error::Error;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshSource {
    pub alias: String,
    pub host: String,
    pub user: Option<String>,
    pub port: String,
    pub key: Option<String>,
    pub remote_path: String,
    pub target: String,
    /// Set when `SYNC_<ALIAS>_DATA_ROOT` or `SYNC_REMOTE_DATA_ROOT` is present.
    pub remote_data_root: Option<PathBuf>,
}

pub fn alias_key(alias: &str) -> String {
    alias
        .chars()
        .map(|c| {
            if c == '-' {
                '_'
            } else {
                c.to_ascii_uppercase()
            }
        })
        .collect()
}

pub fn pick_alias_env(env: &ShopEnv, alias: &str, suffix: &str) -> Option<String> {
    let key = alias_key(alias);
    let specific = format!("SYNC_{key}_{suffix}");
    if let Some(v) = env.get(&specific) {
        return Some(v.to_string());
    }
    env.get(&format!("SYNC_{suffix}")).map(str::to_string)
}

pub fn posix_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn resolve_ssh_source(from: &str, env: &ShopEnv) -> Result<SshSource, Error> {
    let key = alias_key(from);
    let host = pick_alias_env(env, from, "SSH_HOST").unwrap_or_else(|| from.to_string());
    let user = pick_alias_env(env, from, "SSH_USER");
    let port = pick_alias_env(env, from, "SSH_PORT").unwrap_or_else(|| "22".into());
    let keyfile = pick_alias_env(env, from, "SSH_KEY");
    if let Some(k) = &keyfile {
        if !Path::new(k).is_file() {
            return Err(Error::fail(format!("SYNC_SSH_KEY not found: {k}")));
        }
    }
    let remote_path = pick_alias_env(env, from, "REMOTE_PATH").ok_or_else(|| {
        Error::fail(format!(
            "SYNC_REMOTE_PATH (or SYNC_{key}_REMOTE_PATH) is required for --from {from}. Set it in deploy/sync.env (see deploy/sync.env.example)."
        ))
    })?;

    let specific_dr = format!("SYNC_{key}_DATA_ROOT");
    let remote_data_root = env
        .get(&specific_dr)
        .or_else(|| env.get("SYNC_REMOTE_DATA_ROOT"))
        .map(PathBuf::from);

    let target = match user.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(u) => format!("{u}@{host}"),
        None => host.clone(),
    };

    Ok(SshSource {
        alias: from.to_string(),
        host,
        user,
        port,
        key: keyfile,
        remote_path,
        target,
        remote_data_root,
    })
}

/// `ssh` flags without the target (BatchMode; recipes `sync_init_ssh_cmd`).
pub fn ssh_flags(src: &SshSource) -> Vec<String> {
    let mut a = vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-p".into(),
        src.port.clone(),
    ];
    if let Some(k) = &src.key {
        a.push("-o".into());
        a.push("IdentitiesOnly=yes".into());
        a.push("-i".into());
        a.push(k.clone());
    }
    a
}

pub fn ssh_e_opt(src: &SshSource) -> String {
    let mut parts = vec!["ssh".to_string()];
    parts.extend(ssh_flags(src));
    parts.join(" ")
}

pub fn probe_ssh_dry_run_line(src: &SshSource) -> String {
    format!(
        "DRY-RUN ssh {} {} true",
        ssh_flags(src).join(" "),
        src.target
    )
}

pub fn probe_ssh(src: &SshSource) -> Result<(), Error> {
    require_cmd("ssh")?;
    let status = Command::new("ssh")
        .args(ssh_flags(src))
        .arg("-o")
        .arg("ConnectTimeout=15")
        .arg(&src.target)
        .arg("true")
        .status()
        .map_err(|e| Error::fail(format!("could not exec ssh: {e}")))?;
    if !status.success() {
        return Err(Error::fail(format!(
            "SSH to {} failed (BatchMode, no password prompts). Check SYNC_SSH_HOST/USER/PORT/KEY and known_hosts.",
            src.target
        )));
    }
    Ok(())
}

pub fn remote_bash(src: &SshSource, remote_body: &str) -> Result<Output, Error> {
    require_cmd("ssh")?;
    let payload = format!(
        "set -euo pipefail\ncd {cd}\nif [[ -f .env ]]; then set -a; source .env; set +a; fi\nif [[ -f .env.prod ]]; then set -a; source .env.prod; set +a; fi\nexport IMAGE=\"${{IMAGE:-}}\" IMAGE_TAG=\"${{IMAGE_TAG:-latest}}\"\n{body}\n",
        cd = posix_quote(&src.remote_path),
        body = remote_body,
    );
    let mut child = Command::new("ssh")
        .args(ssh_flags(src))
        .arg(&src.target)
        .args(["bash", "-s"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::fail(format!("could not exec ssh: {e}")))?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| Error::fail("internal error: ssh stdin not piped"))?;
        stdin
            .write_all(payload.as_bytes())
            .map_err(|e| Error::fail(format!("could not write ssh payload: {e}")))?;
    }
    child
        .wait_with_output()
        .map_err(|e| Error::fail(format!("ssh failed: {e}")))
}

pub fn remote_dir_exists(src: &SshSource, path: &Path) -> Result<bool, Error> {
    let body = format!("test -d {}", posix_quote(&path.display().to_string()));
    let out = remote_bash(src, &body)?;
    Ok(out.status.success())
}

/// Probe `SYNC_DATA_ROOT` / `SHOPWARE_DATA_ROOT` on the source, else derive.
pub fn resolve_remote_data_root(
    src: &SshSource,
    env: &ShopEnv,
    shop_id: &str,
    dry_run: bool,
) -> Result<(PathBuf, Vec<String>), Error> {
    let mut logs = Vec::new();
    if let Some(p) = &src.remote_data_root {
        return Ok((p.clone(), logs));
    }
    let source_env = source_env_for_remote(&src.alias, env);
    if dry_run {
        let derived = derived_data_root(env, shop_id, &source_env);
        logs.push(format!(
            "DRY-RUN remote SHOPWARE_DATA_ROOT derived {} (probe skipped)",
            derived.display()
        ));
        return Ok((derived, logs));
    }
    let out = remote_bash(
        src,
        r#"printf %s "${SYNC_DATA_ROOT:-${SHOPWARE_DATA_ROOT:-}}""#,
    )?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(Error::fail(format!(
            "SSH probe of remote data root on {} failed: {err}",
            src.target
        )));
    }
    let probed = String::from_utf8_lossy(&out.stdout);
    let probed = probed
        .trim()
        .trim_end_matches('\r')
        .lines()
        .next_back()
        .unwrap_or("")
        .trim();
    if !probed.is_empty() {
        logs.push(format!("Remote bind-mount root: {probed}"));
        return Ok((PathBuf::from(probed), logs));
    }
    let derived = derived_data_root(env, shop_id, &source_env);
    logs.push(format!(
        "Remote bind-mount root derived: {} (shop={shop_id} env={source_env})",
        derived.display()
    ));
    Ok((derived, logs))
}

pub fn resolve_remote_project_name(
    src: &SshSource,
    env: &ShopEnv,
    shop_id: &str,
) -> Result<String, Error> {
    let out = remote_bash(src, r#"printf %s "${COMPOSE_PROJECT_NAME:-}""#)?;
    let n = String::from_utf8_lossy(&out.stdout);
    let n = n.trim().trim_matches('"').trim();
    if !n.is_empty() {
        return Ok(n.to_string());
    }
    let source_env = source_env_for_remote(&src.alias, env);
    Ok(format!("{shop_id}-{source_env}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_with(pairs: &[(&str, &str)]) -> ShopEnv {
        let mut vars = HashMap::new();
        for (k, v) in pairs {
            vars.insert((*k).into(), (*v).into());
        }
        ShopEnv::from_vars(PathBuf::from("/shop"), vars)
    }

    #[test]
    fn alias_key_uppercases_and_hyphens() {
        assert_eq!(alias_key("live"), "LIVE");
        assert_eq!(alias_key("my-vps"), "MY_VPS");
    }

    #[test]
    fn resolve_requires_remote_path() {
        let env = env_with(&[("SHOPWARE_SHOP_ID", "acme")]);
        let err = resolve_ssh_source("live", &env).unwrap_err();
        assert!(err.to_string().contains("SYNC_REMOTE_PATH"), "{err}");
    }

    #[test]
    fn alias_specific_remote_path_and_target() {
        let env = env_with(&[
            ("SYNC_LIVE_REMOTE_PATH", "/opt/shopware/acme"),
            ("SYNC_LIVE_SSH_USER", "deploy"),
            ("SYNC_LIVE_SSH_HOST", "vps.example"),
            ("SYNC_LIVE_DATA_ROOT", "/var/lib/shopware/data/acme/live"),
        ]);
        let src = resolve_ssh_source("live", &env).unwrap();
        assert_eq!(src.target, "deploy@vps.example");
        assert_eq!(src.remote_path, "/opt/shopware/acme");
        assert_eq!(
            src.remote_data_root.as_deref(),
            Some(Path::new("/var/lib/shopware/data/acme/live"))
        );
        assert_eq!(src.port, "22");
    }

    #[test]
    fn posix_quote_escapes_single_quotes() {
        assert_eq!(posix_quote("abc"), "'abc'");
        assert_eq!(posix_quote("a'b"), "'a'\\''b'");
    }
}
