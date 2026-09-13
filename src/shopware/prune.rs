//! Retention prune for timestamped backup artifacts.
//!
//! Standalone `fyrst-cli shopware backup prune` is issue #10. `backup backup`
//! calls [`prune_artifacts`] after a successful artifact (overlay always prunes
//! after backup).

use super::error::Error;
use super::target::{artifact_relpath, ssh_argv, BackupTarget};
use super::volumes::command_exists;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub const DEFAULT_KEEP_DAYS: u32 = 14;
pub const STAMP_LEN: usize = 16; // YYYYMMDDTHHMMSSZ

pub fn parse_keep_days(raw: Option<&str>) -> Result<u32, Error> {
    let s = raw.unwrap_or("").trim();
    if s.is_empty() {
        return Ok(DEFAULT_KEEP_DAYS);
    }
    s.parse::<u32>().map_err(|_| {
        Error::fail(format!(
            "BACKUP_KEEP_DAYS must be a non-negative integer (got {s})"
        ))
    })
}

pub fn is_artifact_stamp(name: &str) -> bool {
    let b = name.as_bytes();
    if b.len() != STAMP_LEN {
        return false;
    }
    b[8] == b'T'
        && b[15] == b'Z'
        && b[..8].iter().all(|c| c.is_ascii_digit())
        && b[9..15].iter().all(|c| c.is_ascii_digit())
}

/// UTC `YYYYMMDDTHHMMSSZ` from Unix seconds (Howard Hinnant civil_from_days).
pub fn utc_stamp(unix_secs: u64) -> String {
    let (y, m, d, hh, mm, ss) = unix_to_utc_parts(unix_secs);
    format!("{y:04}{m:02}{d:02}T{hh:02}{mm:02}{ss:02}Z")
}

fn unix_to_utc_parts(secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let days = (secs / 86400) as i64;
    let rem = (secs % 86400) as u32;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;

    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + i64::from(m <= 2);
    (y as i32, m as u32, d as u32, hour, min, sec)
}

pub fn stamp_is_old(stamp: &str, keep_days: u32, now_unix: u64) -> bool {
    if keep_days == 0 {
        return false;
    }
    if !is_artifact_stamp(stamp) {
        return false;
    }
    let cutoff_secs = now_unix.saturating_sub(u64::from(keep_days) * 86400);
    stamp < utc_stamp(cutoff_secs).as_str()
}

pub struct PruneOpts<'a> {
    pub target: &'a BackupTarget,
    pub shop_id: &'a str,
    pub deploy_env: &'a str,
    pub keep_days: u32,
    pub dry_run: bool,
    pub now_unix: u64,
    pub ssh_key: Option<&'a Path>,
}

pub fn prune_artifacts(opts: &PruneOpts<'_>) -> Result<(), Error> {
    if opts.keep_days == 0 {
        log("BACKUP_KEEP_DAYS=0 — keeping all artifacts");
        return Ok(());
    }

    let cutoff = utc_stamp(
        opts.now_unix
            .saturating_sub(u64::from(opts.keep_days) * 86400),
    );
    log(&format!(
        "Pruning artifacts older than {} daily backups (stamp < {cutoff}) under {}/{}",
        opts.keep_days, opts.shop_id, opts.deploy_env
    ));

    if opts.target.is_ssh() && opts.dry_run {
        log(&format!(
            "DRY-RUN prune remote stamps older than {} days under {}/{} (no SSH)",
            opts.keep_days, opts.shop_id, opts.deploy_env
        ));
        return Ok(());
    }

    let stamps = list_artifact_stamps(opts)?;
    for stamp in stamps {
        if stamp.is_empty() {
            continue;
        }
        if !stamp_is_old(&stamp, opts.keep_days, opts.now_unix) {
            continue;
        }
        let rel = artifact_relpath(opts.shop_id, opts.deploy_env, &stamp);
        let dest = opts.target.artifact_dir(&rel);
        match opts.target {
            BackupTarget::Ssh { .. } => {
                let ssh_target = opts.target.ssh_destination().unwrap();
                log(&format!("Prune {ssh_target}:{dest}"));
                if opts.dry_run {
                    log(&format!("DRY-RUN rm -rf {dest}"));
                    continue;
                }
                remote_rm_rf(opts.target, opts.ssh_key, &dest)?;
            }
            BackupTarget::Local { .. } => {
                log(&format!("Prune {dest}"));
                if opts.dry_run {
                    log(&format!("DRY-RUN rm -rf {dest}"));
                    continue;
                }
                let path = Path::new(&dest);
                if path.exists() {
                    fs::remove_dir_all(path).map_err(|e| {
                        Error::fail(format!("cannot rm -rf {}: {e}", path.display()))
                    })?;
                }
            }
        }
    }
    Ok(())
}

fn list_artifact_stamps(opts: &PruneOpts<'_>) -> Result<Vec<String>, Error> {
    match opts.target {
        BackupTarget::Local { path } => {
            let dir = path.join(opts.shop_id).join(opts.deploy_env);
            if !dir.is_dir() {
                return Ok(Vec::new());
            }
            let mut names = Vec::new();
            for entry in fs::read_dir(&dir)
                .map_err(|e| Error::fail(format!("cannot list {}: {e}", dir.display())))?
            {
                let entry = entry.map_err(|e| Error::fail(format!("cannot read dirent: {e}")))?;
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
            names.sort();
            Ok(names)
        }
        BackupTarget::Ssh { .. } => {
            let prefix = opts
                .target
                .artifact_dir(&format!("{}/{}", opts.shop_id, opts.deploy_env));
            remote_ls(opts.target, opts.ssh_key, &prefix)
        }
    }
}

fn remote_ls(
    target: &BackupTarget,
    ssh_key: Option<&Path>,
    dir: &str,
) -> Result<Vec<String>, Error> {
    let payload = format!("ls -1 {} 2>/dev/null || true", sh_quote(dir));
    let out = remote_sh(target, ssh_key, &payload)?;
    Ok(out
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

fn remote_rm_rf(target: &BackupTarget, ssh_key: Option<&Path>, dest: &str) -> Result<(), Error> {
    let payload = format!("rm -rf {}", sh_quote(dest));
    remote_sh(target, ssh_key, &payload)?;
    Ok(())
}

pub fn remote_sh(
    target: &BackupTarget,
    ssh_key: Option<&Path>,
    payload: &str,
) -> Result<String, Error> {
    let (dest, port) = match target {
        BackupTarget::Ssh { port, .. } => (target.ssh_destination().unwrap(), port.as_str()),
        BackupTarget::Local { .. } => {
            return Err(Error::fail("remote_sh called for a local BACKUP_TARGET"));
        }
    };
    if !command_exists("ssh") {
        return Err(Error::fail("ssh is required to reach an SSH BACKUP_TARGET"));
    }
    let args = ssh_argv(port, ssh_key);
    let mut cmd = Command::new("ssh");
    cmd.args(&args).arg(&dest).args(["bash", "-s"]);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::fail(format!("could not exec ssh: {e}")))?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::fail("ssh stdin unavailable"))?;
        stdin
            .write_all(payload.as_bytes())
            .and_then(|_| stdin.write_all(b"\n"))
            .map_err(|e| Error::fail(format!("could not write ssh payload: {e}")))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| Error::fail(format!("ssh wait failed: {e}")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(Error::fail(format!(
            "SSH to {dest} failed (BatchMode). Check BACKUP_TARGET / BACKUP_SSH_KEY. {err}"
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn log(msg: &str) {
    println!("==> {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn keep_days_parse() {
        assert_eq!(parse_keep_days(None).unwrap(), 14);
        assert_eq!(parse_keep_days(Some("")).unwrap(), 14);
        assert_eq!(parse_keep_days(Some("0")).unwrap(), 0);
        assert_eq!(parse_keep_days(Some("30")).unwrap(), 30);
        assert!(parse_keep_days(Some("-1"))
            .unwrap_err()
            .to_string()
            .contains("BACKUP_KEEP_DAYS"));
        assert!(parse_keep_days(Some("nope"))
            .unwrap_err()
            .to_string()
            .contains("BACKUP_KEEP_DAYS"));
    }

    #[test]
    fn stamp_shape() {
        assert!(is_artifact_stamp("20260913T020000Z"));
        assert!(!is_artifact_stamp("latest"));
        assert!(!is_artifact_stamp("2026-09-13T020000Z"));
        assert!(!is_artifact_stamp("20260913T020000"));
        assert!(!is_artifact_stamp(""));
    }

    #[test]
    fn unix_epoch_stamp() {
        assert_eq!(utc_stamp(0), "19700101T000000Z");
        assert_eq!(utc_stamp(1700000000), "20231114T221320Z");
    }

    #[test]
    fn cutoff_math_and_keep_forever() {
        let now = 1_757_781_000; // some 2026 unix
        assert!(!stamp_is_old("20260913T020000Z", 0, now));
        assert!(stamp_is_old("20200101T000000Z", 14, now));
        assert!(!stamp_is_old("notes.txt", 14, now));
        let current = utc_stamp(now);
        assert!(!stamp_is_old(&current, 14, now));
        let just_inside = utc_stamp(now - 13 * 86400);
        assert!(!stamp_is_old(&just_inside, 14, now));
        let just_outside = utc_stamp(now - 15 * 86400);
        assert!(stamp_is_old(&just_outside, 14, now));
    }

    #[test]
    fn prune_deletes_old_local_stamps_only() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("fyrst-cli-prune-{}-{nanos}", std::process::id()));
        let env_dir = root.join("acme").join("live");
        fs::create_dir_all(env_dir.join("20200101T000000Z")).unwrap();
        fs::write(env_dir.join("20200101T000000Z/old.txt"), b"x").unwrap();
        fs::create_dir_all(env_dir.join("notes.txt")).unwrap();
        let now = 1_757_781_000u64;
        let keep = utc_stamp(now);
        fs::create_dir_all(env_dir.join(&keep)).unwrap();
        fs::write(env_dir.join(&keep).join("new.txt"), b"y").unwrap();

        let target = BackupTarget::Local { path: root.clone() };
        prune_artifacts(&PruneOpts {
            target: &target,
            shop_id: "acme",
            deploy_env: "live",
            keep_days: 14,
            dry_run: false,
            now_unix: now,
            ssh_key: None,
        })
        .unwrap();

        assert!(!env_dir.join("20200101T000000Z").exists());
        assert!(env_dir.join("notes.txt").exists());
        assert!(env_dir.join(&keep).exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn prune_keep_forever_is_noop() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("fyrst-cli-prune0-{}-{nanos}", std::process::id()));
        let env_dir = root.join("acme").join("live");
        fs::create_dir_all(env_dir.join("20200101T000000Z")).unwrap();
        let target = BackupTarget::Local { path: root.clone() };
        prune_artifacts(&PruneOpts {
            target: &target,
            shop_id: "acme",
            deploy_env: "live",
            keep_days: 0,
            dry_run: false,
            now_unix: 1_757_781_000,
            ssh_key: None,
        })
        .unwrap();
        assert!(env_dir.join("20200101T000000Z").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn prune_dry_run_does_not_delete() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("fyrst-cli-prunedry-{}-{nanos}", std::process::id()));
        let env_dir = root.join("acme").join("live");
        fs::create_dir_all(env_dir.join("20200101T000000Z")).unwrap();
        let target = BackupTarget::Local { path: root.clone() };
        prune_artifacts(&PruneOpts {
            target: &target,
            shop_id: "acme",
            deploy_env: "live",
            keep_days: 14,
            dry_run: true,
            now_unix: 1_757_781_000,
            ssh_key: None,
        })
        .unwrap();
        assert!(env_dir.join("20200101T000000Z").exists());
        let _ = fs::remove_dir_all(&root);
    }
}
