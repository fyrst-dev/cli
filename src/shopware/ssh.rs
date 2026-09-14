//! SSH source resolution for remote `--from`.
//!
//! Host / user / key come from `SHOPWARE_SSH_*`. Host defaults to the
//! `--from` alias (`live` → `~/.ssh/config` `Host live`) when unset.

use super::env::{remote_data_root, require_cmd, require_shop_id, ShopEnv};
use super::error::Error;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshSource {
    pub alias: String,
    pub host: String,
    pub user: Option<String>,
    pub port: String,
    pub key: Option<String>,
    pub remote_path: Option<String>,
    pub target: String,
    pub remote_data_root: Option<PathBuf>,
}

pub fn posix_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn resolve_keyfile(env: &ShopEnv) -> Result<Option<String>, Error> {
    let Some(k) = env.get("SHOPWARE_SSH_KEY") else {
        return Ok(None);
    };
    let p = Path::new(k);
    let resolved = if p.is_absolute() {
        p.to_path_buf()
    } else {
        env.compose_dir.join(p)
    };
    if !resolved.is_file() {
        return Err(Error::fail(format!("SHOPWARE_SSH_KEY not found: {k}")));
    }
    Ok(Some(resolved.to_string_lossy().into_owned()))
}

pub fn resolve_ssh_source(from: &str, env: &ShopEnv) -> Result<SshSource, Error> {
    let host = env.get("SHOPWARE_SSH_HOST").unwrap_or(from).to_string();
    let user = env.get("SHOPWARE_SSH_USER").map(str::to_string);
    let keyfile = resolve_keyfile(env)?;
    let remote_data = env.get("SHOPWARE_REMOTE_DATA_ROOT").map(PathBuf::from);

    let target = match user.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(u) => format!("{u}@{host}"),
        None => host.clone(),
    };

    Ok(SshSource {
        alias: from.to_string(),
        host,
        user,
        port: "22".into(),
        key: keyfile,
        remote_path: None,
        target,
        remote_data_root: remote_data,
    })
}

/// `ssh` flags without the target (BatchMode).
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

pub fn ssh_argv_vec(src: &SshSource) -> Vec<String> {
    let mut cmd = vec!["ssh".to_string()];
    cmd.extend(ssh_flags(src));
    cmd
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
            "SSH to {} failed (BatchMode, no password prompts). Check Host {} in ~/.ssh/config, SHOPWARE_SSH_HOST/USER/KEY, and known_hosts.",
            src.target, src.alias
        )));
    }
    Ok(())
}

pub fn spawn_remote_bash(src: &SshSource, remote_body: &str) -> Result<Child, Error> {
    require_cmd("ssh")?;
    let payload = match src
        .remote_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(path) => format!(
            "set -euo pipefail\ncd {cd}\nif [[ -f .env ]]; then set -a; source .env; set +a; fi\nif [[ -f .env.prod ]]; then set -a; source .env.prod; set +a; fi\nexport IMAGE=\"${{IMAGE:-}}\" IMAGE_TAG=\"${{IMAGE_TAG:-latest}}\"\n{body}\n",
            cd = posix_quote(path),
            body = remote_body,
        ),
        None => format!("set -euo pipefail\n{remote_body}\n"),
    };
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
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::fail("internal error: ssh stdin not piped"))?;
        stdin
            .write_all(payload.as_bytes())
            .map_err(|e| Error::fail(format!("could not write ssh payload: {e}")))?;
    }
    Ok(child)
}

pub fn remote_bash(src: &SshSource, remote_body: &str) -> Result<Output, Error> {
    spawn_remote_bash(src, remote_body)?
        .wait_with_output()
        .map_err(|e| Error::fail(format!("ssh failed: {e}")))
}

pub fn remote_dir_exists(src: &SshSource, path: &Path) -> Result<bool, Error> {
    let body = format!("test -d {}", posix_quote(&path.display().to_string()));
    let out = remote_bash(src, &body)?;
    Ok(out.status.success())
}

/// `SHOPWARE_REMOTE_DATA_ROOT`, else `{data_base}/{shop_id}/live`.
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
    let derived = remote_data_root(env, shop_id);
    if dry_run {
        logs.push(format!(
            "DRY-RUN remote data root derived {} (probe skipped)",
            derived.display()
        ));
    } else {
        logs.push(format!(
            "Remote bind-mount root derived: {} (shop={shop_id} env=live)",
            derived.display()
        ));
    }
    Ok((derived, logs))
}

pub fn resolve_remote_project_name(
    src: &SshSource,
    env: &ShopEnv,
    shop_id: &str,
) -> Result<String, Error> {
    if src.remote_path.is_some() {
        let out = remote_bash(src, r#"printf %s "${COMPOSE_PROJECT_NAME:-}""#)?;
        let n = String::from_utf8_lossy(&out.stdout);
        let n = n.trim().trim_matches('"').trim();
        if !n.is_empty() {
            return Ok(n.to_string());
        }
    }
    let _ = require_shop_id(env);
    Ok(format!("{shop_id}-live"))
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
    fn host_defaults_to_from_alias() {
        let env = env_with(&[("SHOPWARE_SHOP_ID", "acme")]);
        let src = resolve_ssh_source("live", &env).unwrap();
        assert_eq!(src.host, "live");
        assert_eq!(src.target, "live");
        assert_eq!(src.port, "22");
        assert!(src.remote_path.is_none());
        assert!(src.remote_data_root.is_none());
    }

    #[test]
    fn shopware_ssh_trio() {
        let env = env_with(&[
            ("SHOPWARE_SSH_HOST", "vps.example"),
            ("SHOPWARE_SSH_USER", "deploy"),
            (
                "SHOPWARE_REMOTE_DATA_ROOT",
                "/var/lib/shopware/data/acme/live",
            ),
        ]);
        let src = resolve_ssh_source("live", &env).unwrap();
        assert_eq!(src.target, "deploy@vps.example");
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
