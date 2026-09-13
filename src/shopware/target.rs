//! Parse `BACKUP_TARGET` (local path or SSH), matching `backup-runtime.sh`.

use super::error::Error;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupTarget {
    Local {
        path: PathBuf,
    },
    Ssh {
        user: Option<String>,
        host: String,
        port: String,
        path: String,
    },
}

impl BackupTarget {
    pub fn is_ssh(&self) -> bool {
        matches!(self, Self::Ssh { .. })
    }

    pub fn ssh_destination(&self) -> Option<String> {
        match self {
            Self::Ssh { user, host, .. } => Some(match user {
                Some(u) if !u.is_empty() => format!("{u}@{host}"),
                _ => host.clone(),
            }),
            Self::Local { .. } => None,
        }
    }

    pub fn ssh_port(&self) -> Option<&str> {
        match self {
            Self::Ssh { port, .. } => Some(port.as_str()),
            Self::Local { .. } => None,
        }
    }

    pub fn prefix_path(&self) -> &str {
        match self {
            Self::Local { path } => path.to_str().unwrap_or(""),
            Self::Ssh { path, .. } => path.as_str(),
        }
    }

    /// `$BACKUP_TARGET/$shop_id/$deploy_env/$stamp` (local path or remote path).
    pub fn artifact_dir(&self, rel: &str) -> String {
        let prefix = self.prefix_path().trim_end_matches('/');
        format!("{prefix}/{rel}")
    }
}

pub fn artifact_relpath(shop_id: &str, deploy_env: &str, stamp: &str) -> String {
    format!("{shop_id}/{deploy_env}/{stamp}")
}

/// `default_port` is `BACKUP_SSH_PORT` (overlay default `22`), overridden by `ssh://`.
pub fn parse_backup_target(raw: &str, default_port: &str) -> Result<BackupTarget, Error> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(Error::fail(
            "BACKUP_TARGET is required (local path, second disk, or user@host:/path). See deploy/backup.env.example.",
        ));
    }

    let port = if default_port.trim().is_empty() {
        "22"
    } else {
        default_port.trim()
    };

    if let Some(rest) = raw.strip_prefix("ssh://") {
        return parse_ssh_url(rest, port);
    }

    // user@host:/path or host:/path (not an absolute /path or ./relative).
    if raw.contains(':') && !raw.starts_with('/') && !raw.starts_with("./") {
        return parse_scp_form(raw, port);
    }

    Ok(BackupTarget::Local {
        path: PathBuf::from(raw),
    })
}

fn parse_ssh_url(rest: &str, default_port: &str) -> Result<BackupTarget, Error> {
    let (hostpart, pathpart) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => {
            return Err(Error::fail(
                "ssh:// BACKUP_TARGET must include an absolute path (ssh://user@host:22/var/backups/shopware)",
            ));
        }
    };

    let (user, hostpart) = if let Some((u, h)) = hostpart.split_once('@') {
        (Some(u.to_string()).filter(|s| !s.is_empty()), h)
    } else {
        (None, hostpart)
    };

    if hostpart.contains("]:") {
        return Err(Error::fail(
            "IPv6 ssh:// with port: use user@[::1]:22/path via user@host:/path form instead",
        ));
    }

    let (host, port) = if hostpart.contains(':') {
        let (h, p) = hostpart.split_once(':').unwrap();
        (h.to_string(), p.to_string())
    } else {
        (hostpart.to_string(), default_port.to_string())
    };

    if host.is_empty() {
        return Err(Error::fail("BACKUP_TARGET ssh:// is missing a host"));
    }

    Ok(BackupTarget::Ssh {
        user,
        host,
        port,
        path: pathpart,
    })
}

fn parse_scp_form(raw: &str, default_port: &str) -> Result<BackupTarget, Error> {
    let (left, path) = raw.split_once(':').unwrap();
    let mut path = path.to_string();
    if !path.starts_with('/') {
        path = format!("/{path}");
    }
    let (user, host) = if let Some((u, h)) = left.split_once('@') {
        (Some(u.to_string()).filter(|s| !s.is_empty()), h.to_string())
    } else {
        (None, left.to_string())
    };
    if host.is_empty() {
        return Err(Error::fail("BACKUP_TARGET SSH host is empty"));
    }
    Ok(BackupTarget::Ssh {
        user,
        host,
        port: default_port.to_string(),
        path,
    })
}

pub fn resolve_local_target_path(path: &Path, compose_dir: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        compose_dir.join(path)
    }
}

pub fn ssh_argv(port: &str, identity: Option<&Path>) -> Vec<String> {
    let mut a = vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ConnectTimeout=15".into(),
        "-p".into(),
        port.to_string(),
    ];
    if let Some(key) = identity {
        a.push("-o".into());
        a.push("IdentitiesOnly=yes".into());
        a.push("-i".into());
        a.push(key.display().to_string());
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_is_error() {
        let err = parse_backup_target("", "22").unwrap_err();
        assert!(err.to_string().contains("BACKUP_TARGET"), "{err}");
        let err = parse_backup_target("   ", "22").unwrap_err();
        assert!(err.to_string().contains("BACKUP_TARGET"), "{err}");
    }

    #[test]
    fn local_absolute_and_relative() {
        match parse_backup_target("/var/backups/shopware", "22").unwrap() {
            BackupTarget::Local { path } => {
                assert_eq!(path, PathBuf::from("/var/backups/shopware"));
            }
            other => panic!("{other:?}"),
        }
        match parse_backup_target("./backups", "22").unwrap() {
            BackupTarget::Local { path } => assert_eq!(path, PathBuf::from("./backups")),
            other => panic!("{other:?}"),
        }
        match parse_backup_target("backups/shopware", "22").unwrap() {
            BackupTarget::Local { path } => assert_eq!(path, PathBuf::from("backups/shopware")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn scp_user_host_path() {
        match parse_backup_target("backup@backup-host:/var/backups/shopware", "22").unwrap() {
            BackupTarget::Ssh {
                user,
                host,
                port,
                path,
            } => {
                assert_eq!(user.as_deref(), Some("backup"));
                assert_eq!(host, "backup-host");
                assert_eq!(port, "22");
                assert_eq!(path, "/var/backups/shopware");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn scp_host_only_and_relative_remote_path() {
        match parse_backup_target("backup-host:var/backups", "2222").unwrap() {
            BackupTarget::Ssh {
                user,
                host,
                port,
                path,
            } => {
                assert!(user.is_none());
                assert_eq!(host, "backup-host");
                assert_eq!(port, "2222");
                assert_eq!(path, "/var/backups");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn ssh_url_with_port() {
        match parse_backup_target("ssh://backup@backup-host:22/var/backups/shopware", "99").unwrap()
        {
            BackupTarget::Ssh {
                user,
                host,
                port,
                path,
            } => {
                assert_eq!(user.as_deref(), Some("backup"));
                assert_eq!(host, "backup-host");
                assert_eq!(port, "22");
                assert_eq!(path, "/var/backups/shopware");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn ssh_url_default_port_and_no_user() {
        match parse_backup_target("ssh://backup-host/abs/path", "2222").unwrap() {
            BackupTarget::Ssh {
                user,
                host,
                port,
                path,
            } => {
                assert!(user.is_none());
                assert_eq!(host, "backup-host");
                assert_eq!(port, "2222");
                assert_eq!(path, "/abs/path");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn ssh_url_requires_absolute_path() {
        let err = parse_backup_target("ssh://user@host", "22").unwrap_err();
        assert!(err.to_string().contains("absolute path"), "{err}");
    }

    #[test]
    fn ipv6_ssh_url_with_port_refused() {
        let err = parse_backup_target("ssh://user@[::1]:22/var/backups", "22").unwrap_err();
        assert!(err.to_string().contains("IPv6"), "{err}");
    }

    #[test]
    fn artifact_relpath_layout() {
        assert_eq!(
            artifact_relpath("acme", "live", "20260913T020000Z"),
            "acme/live/20260913T020000Z"
        );
    }
}
