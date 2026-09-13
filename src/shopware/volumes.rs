//! Bind-mount / named-volume snapshot (recipes `deploy/lib/sync-volumes.sh`).
//!
//! Layout: `--snapshot-dir/data/<item>/` for bind-mount rsync, or
//! `--snapshot-dir/volumes/<item>.tar.gz` when the bind-mount is missing
//! (named volume via `SYNC_ARCHIVE_IMAGE`, default `alpine:3.20`).
//! Never runs shopware-cli.

use super::env::{archive_image, derive_project_name, have_cmd, ShopEnv};
use super::error::Error;
use super::mysql::require_docker;
use super::ssh::{
    posix_quote, remote_bash, remote_dir_exists, resolve_remote_project_name, ssh_e_opt, SshSource,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VolumeTransport {
    LocalBind {
        src: PathBuf,
        dest: PathBuf,
    },
    LocalNamedVolume {
        volume: String,
        tar_out: PathBuf,
        archive_image: String,
        bind_src: PathBuf,
        data_root: PathBuf,
    },
    Remote {
        ssh: SshSource,
        remote_src: PathBuf,
        dest: PathBuf,
        snapshot_dir: PathBuf,
        shop_id: String,
        archive_image: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeAction {
    pub item: String,
    pub transport: VolumeTransport,
    pub logs: Vec<String>,
}

pub fn bind_item_dir(root: &Path, logical: &str) -> PathBuf {
    root.join(logical)
}

pub fn volume_docker_name(project: &str, logical: &str) -> String {
    format!("{project}_{logical}")
}

pub fn plan_local_volumes(
    items: &[String],
    data_root: &Path,
    snapshot_dir: &Path,
    env: &ShopEnv,
    dry_run: bool,
) -> Result<Vec<VolumeAction>, Error> {
    let mut actions = Vec::new();
    let mut project: Option<String> = None;
    let image = archive_image(env);
    for item in items {
        let src = bind_item_dir(data_root, item);
        let dest = snapshot_dir.join("data").join(item);
        if src.is_dir() {
            let mut logs = vec![format!(
                "Snapshot bind mount {} → {}",
                src.display(),
                dest.display()
            )];
            if dry_run {
                logs.push(format!(
                    "DRY-RUN rsync {}/ {}/",
                    src.display(),
                    dest.display()
                ));
            }
            actions.push(VolumeAction {
                item: item.clone(),
                transport: VolumeTransport::LocalBind { src, dest },
                logs,
            });
            continue;
        }
        if project.is_none() {
            project = Some(derive_project_name(env)?.0);
        }
        let project = project.as_deref().unwrap();
        let volume = volume_docker_name(project, item);
        let tar_out = snapshot_dir.join("volumes").join(format!("{item}.tar.gz"));
        let mut logs = vec![
            format!("Bind mount {} missing; trying named volume", src.display()),
            format!(
                "Archiving named volume {volume} → {} (bind-mount fallback)",
                tar_out.display()
            ),
        ];
        if dry_run {
            logs.push(format!(
                "DRY-RUN docker run --rm -v {volume}:/from:ro -v {}:/to {image} tar",
                snapshot_dir.join("volumes").display()
            ));
        }
        actions.push(VolumeAction {
            item: item.clone(),
            transport: VolumeTransport::LocalNamedVolume {
                volume,
                tar_out,
                archive_image: image.clone(),
                bind_src: src,
                data_root: data_root.to_path_buf(),
            },
            logs,
        });
    }
    Ok(actions)
}

pub fn plan_remote_volumes(
    items: &[String],
    remote_data_root: &Path,
    snapshot_dir: &Path,
    ssh: &SshSource,
    shop_id: &str,
    env: &ShopEnv,
    dry_run: bool,
) -> Vec<VolumeAction> {
    let image = archive_image(env);
    items
        .iter()
        .map(|item| {
            let remote_src = bind_item_dir(remote_data_root, item);
            let dest = snapshot_dir.join("data").join(item);
            let mut logs = Vec::new();
            if dry_run {
                logs.push(format!(
                    "DRY-RUN rsync {}:{}/ → {}/",
                    ssh.target,
                    remote_src.display(),
                    dest.display()
                ));
            }
            VolumeAction {
                item: item.clone(),
                transport: VolumeTransport::Remote {
                    ssh: ssh.clone(),
                    remote_src,
                    dest,
                    snapshot_dir: snapshot_dir.to_path_buf(),
                    shop_id: shop_id.to_string(),
                    archive_image: image.clone(),
                },
                logs,
            }
        })
        .collect()
}

pub fn execute_volume(action: &VolumeAction, dry_run: bool, env: &ShopEnv) -> Result<(), Error> {
    for line in &action.logs {
        println!("==> {line}");
    }
    if dry_run {
        return Ok(());
    }
    match &action.transport {
        VolumeTransport::LocalBind { src, dest } => snapshot_bind_local(src, dest),
        VolumeTransport::LocalNamedVolume {
            volume,
            tar_out,
            archive_image,
            bind_src,
            data_root,
        } => archive_volume_local(volume, tar_out, archive_image, bind_src, data_root),
        VolumeTransport::Remote {
            ssh,
            remote_src,
            dest,
            snapshot_dir,
            shop_id,
            archive_image,
        } => snapshot_bind_remote(
            &action.item,
            ssh,
            remote_src,
            dest,
            snapshot_dir,
            shop_id,
            archive_image,
            env,
        ),
    }
}

fn snapshot_bind_local(src: &Path, dest: &Path) -> Result<(), Error> {
    if have_cmd("rsync") {
        rsync_local_trees(src, dest)
    } else {
        println!(
            "==> rsync not installed; copying bind-mount tree {} → {} without rsync",
            src.display(),
            dest.display()
        );
        copy_tree_replace(src, dest)
    }
}

fn rsync_local_trees(src: &Path, dest: &Path) -> Result<(), Error> {
    fs::create_dir_all(dest)
        .map_err(|e| Error::fail(format!("cannot create {}: {e}", dest.display())))?;
    let status = Command::new("rsync")
        .args(["-aH", "--delete", "--numeric-ids"])
        .arg(trailing_slash(src))
        .arg(trailing_slash(dest))
        .status()
        .map_err(|e| Error::fail(format!("could not exec rsync: {e}")))?;
    if !status.success() {
        return Err(Error::fail(format!(
            "rsync {} → {} failed",
            src.display(),
            dest.display()
        )));
    }
    Ok(())
}

fn archive_volume_local(
    volume: &str,
    tar_out: &Path,
    archive_image: &str,
    bind_src: &Path,
    data_root: &Path,
) -> Result<(), Error> {
    require_docker()?;
    let inspect = Command::new("docker")
        .args(["volume", "inspect", volume])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| Error::fail(format!("could not exec docker volume inspect: {e}")))?;
    if !inspect.success() {
        return Err(Error::fail(format!(
            "Named volume '{volume}' not found and bind-mount {} is missing. mkdir -p {}/{{files,media,thumbnail,theme,sitemap}} && chown 82:82 (see deploy/README.md).",
            bind_src.display(),
            data_root.display()
        )));
    }
    let volumes_dir = tar_out
        .parent()
        .ok_or_else(|| Error::fail(format!("invalid volume archive path {}", tar_out.display())))?;
    fs::create_dir_all(volumes_dir)
        .map_err(|e| Error::fail(format!("cannot create {}: {e}", volumes_dir.display())))?;
    let vol_mount = format!("{volume}:/from:ro");
    let to_mount = format!("{}:/to", volumes_dir.display());
    let tar_name = tar_out
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Error::fail("volume archive name is not utf-8"))?;
    let tar_in_container = format!("/to/{tar_name}");
    let status = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &vol_mount,
            "-v",
            &to_mount,
            archive_image,
            "tar",
            "-C",
            "/from",
            "-czf",
            &tar_in_container,
            ".",
        ])
        .status()
        .map_err(|e| Error::fail(format!("could not exec docker run tar: {e}")))?;
    if !status.success() {
        return Err(Error::fail(format!(
            "docker tar of named volume {volume} failed"
        )));
    }
    gzip_test(tar_out)
}

fn snapshot_bind_remote(
    item: &str,
    ssh: &SshSource,
    remote_src: &Path,
    dest: &Path,
    snapshot_dir: &Path,
    shop_id: &str,
    archive_image: &str,
    env: &ShopEnv,
) -> Result<(), Error> {
    if remote_dir_exists(ssh, remote_src)? {
        println!(
            "==> Snapshot remote bind mount {}:{} → {}",
            ssh.target,
            remote_src.display(),
            dest.display()
        );
        if have_cmd("rsync") {
            return rsync_from_remote_tree(ssh, remote_src, dest);
        }
        let tar_out = snapshot_dir.join("volumes").join(format!("{item}.tar.gz"));
        fs::create_dir_all(tar_out.parent().unwrap_or(snapshot_dir))
            .map_err(|e| Error::fail(format!("cannot create volume archive dir: {e}")))?;
        archive_remote_bind_via_tar(ssh, remote_src, &tar_out, archive_image)?;
        return gzip_test(&tar_out);
    }
    println!(
        "==> Remote bind mount {} missing; trying named volume",
        remote_src.display()
    );
    archive_volume_remote(item, ssh, snapshot_dir, shop_id, archive_image, env)
}

fn rsync_from_remote_tree(ssh: &SshSource, remote_dir: &Path, dest: &Path) -> Result<(), Error> {
    fs::create_dir_all(dest)
        .map_err(|e| Error::fail(format!("cannot create {}: {e}", dest.display())))?;
    let e = ssh_e_opt(ssh);
    let src = format!("{}:{}/", ssh.target, remote_dir.display());
    let status = Command::new("rsync")
        .args(["-azH", "--delete", "--numeric-ids", "-e"])
        .arg(&e)
        .arg(&src)
        .arg(trailing_slash(dest))
        .status()
        .map_err(|e| Error::fail(format!("could not exec rsync: {e}")))?;
    if !status.success() {
        return Err(Error::fail(format!(
            "rsync {} → {} failed",
            src,
            dest.display()
        )));
    }
    Ok(())
}

fn archive_remote_bind_via_tar(
    ssh: &SshSource,
    remote_src: &Path,
    tar_out: &Path,
    archive_image: &str,
) -> Result<(), Error> {
    let body = format!(
        "docker run --rm -v {src}:/from:ro {image} tar -C /from -czf - .",
        src = posix_quote(&remote_src.display().to_string()),
        image = posix_quote(archive_image),
    );
    stream_remote_tar(ssh, &body, tar_out)
}

fn archive_volume_remote(
    item: &str,
    ssh: &SshSource,
    snapshot_dir: &Path,
    shop_id: &str,
    archive_image: &str,
    env: &ShopEnv,
) -> Result<(), Error> {
    let project = resolve_remote_project_name(ssh, env, shop_id)?;
    let volume = volume_docker_name(&project, item);
    let tar_out = snapshot_dir.join("volumes").join(format!("{item}.tar.gz"));
    println!(
        "==> Archiving remote named volume {volume} on {} → {} (bind-mount fallback)",
        ssh.target,
        tar_out.display()
    );
    fs::create_dir_all(tar_out.parent().unwrap_or(snapshot_dir))
        .map_err(|e| Error::fail(format!("cannot create volume archive dir: {e}")))?;
    let vol_q = posix_quote(&volume);
    let img_q = posix_quote(archive_image);
    let body = format!(
        "if ! docker volume inspect {vol} >/dev/null 2>&1; then\n  echo \"Named volume {volume} not found on source (project {project}).\" >&2\n  exit 1\nfi\ndocker run --rm -v {vol}:/from:ro {image} tar -C /from -czf - .",
        vol = vol_q,
        image = img_q,
    );
    stream_remote_tar(ssh, &body, &tar_out)?;
    if tar_out.metadata().map(|m| m.len()).unwrap_or(0) == 0 {
        return Err(Error::fail(format!(
            "Remote volume archive for {item} was empty"
        )));
    }
    gzip_test(&tar_out)
}

fn stream_remote_tar(ssh: &SshSource, remote_body: &str, tar_out: &Path) -> Result<(), Error> {
    let out = remote_bash(ssh, remote_body)?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(Error::fail(format!(
            "remote tar on {} failed: {err}",
            ssh.target
        )));
    }
    fs::write(tar_out, &out.stdout)
        .map_err(|e| Error::fail(format!("cannot write {}: {e}", tar_out.display())))
}

fn gzip_test(path: &Path) -> Result<(), Error> {
    if !have_cmd("gzip") {
        return Ok(());
    }
    let status = Command::new("gzip")
        .arg("-t")
        .arg(path)
        .status()
        .map_err(|e| Error::fail(format!("could not exec gzip -t: {e}")))?;
    if !status.success() {
        return Err(Error::fail(format!(
            "volume archive {} failed gzip -t",
            path.display()
        )));
    }
    Ok(())
}

fn trailing_slash(p: &Path) -> std::ffi::OsString {
    let mut s = p.as_os_str().to_os_string();
    s.push("/");
    s
}

fn copy_tree_replace(src: &Path, dest: &Path) -> Result<(), Error> {
    if dest.exists() {
        fs::remove_dir_all(dest)
            .map_err(|e| Error::fail(format!("cannot replace {}: {e}", dest.display())))?;
    }
    copy_tree(src, dest)
}

fn copy_tree(src: &Path, dest: &Path) -> Result<(), Error> {
    fs::create_dir_all(dest)
        .map_err(|e| Error::fail(format!("cannot create {}: {e}", dest.display())))?;
    let entries = fs::read_dir(src)
        .map_err(|e| Error::fail(format!("cannot read {}: {e}", src.display())))?;
    for ent in entries {
        let ent = ent.map_err(|e| Error::fail(format!("cannot read {}: {e}", src.display())))?;
        let from = ent.path();
        let to = dest.join(ent.file_name());
        let ft = ent
            .file_type()
            .map_err(|e| Error::fail(format!("cannot stat {}: {e}", from.display())))?;
        if ft.is_dir() {
            copy_tree(&from, &to)?;
        } else if ft.is_file() {
            fs::copy(&from, &to).map_err(|e| {
                Error::fail(format!(
                    "cannot copy {} → {}: {e}",
                    from.display(),
                    to.display()
                ))
            })?;
        } else if ft.is_symlink() {
            let target = fs::read_link(&from)
                .map_err(|e| Error::fail(format!("cannot read symlink {}: {e}", from.display())))?;
            #[cfg(unix)]
            {
                if to.symlink_metadata().is_ok() {
                    let _ = fs::remove_file(&to);
                }
                std::os::unix::fs::symlink(&target, &to)
                    .map_err(|e| Error::fail(format!("cannot symlink {}: {e}", to.display())))?;
            }
            #[cfg(not(unix))]
            {
                return Err(Error::fail(format!(
                    "cannot copy symlink {} on this platform",
                    from.display()
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!(
            "fyrst-cli-vol-{prefix}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn local_bind_plan_prints_rsync_when_dir_exists() {
        let root = temp_dir("bind");
        let media = root.join("media");
        fs::create_dir_all(&media).unwrap();
        fs::write(media.join("a.txt"), b"x").unwrap();
        let snap = root.join("snap");
        let env = ShopEnv::from_vars(PathBuf::from("/shop"), HashMap::new());
        let actions = plan_local_volumes(&["media".into()], &root, &snap, &env, true).unwrap();
        assert_eq!(actions.len(), 1);
        let text = actions[0].logs.join("\n");
        assert!(text.contains("DRY-RUN rsync"), "{text}");
        assert!(text.contains("data/media"), "{text}");
        assert!(!text.contains("shopware-cli"), "{text}");
        assert!(!text.contains("docker run"), "{text}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_bind_plans_named_volume_tar() {
        let root = temp_dir("novol");
        let snap = root.join("snap");
        let mut vars = HashMap::new();
        vars.insert("COMPOSE_PROJECT_NAME".into(), "acme-staging".into());
        let env = ShopEnv::from_vars(PathBuf::from("/shop"), vars);
        let actions =
            plan_local_volumes(&["media".into()], &root.join("data"), &snap, &env, true).unwrap();
        let text = actions[0].logs.join("\n");
        assert!(text.contains("named volume"), "{text}");
        assert!(text.contains("acme-staging_media"), "{text}");
        assert!(text.contains("DRY-RUN docker run"), "{text}");
        assert!(text.contains("alpine:3.20"), "{text}");
        assert!(!text.contains("shopware-cli"), "{text}");
        assert!(!text.contains("project dump"), "{text}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn copy_tree_replace_mirrors_files() {
        let root = temp_dir("copy");
        let src = root.join("src");
        let dest = root.join("dest");
        fs::create_dir_all(src.join("nested")).unwrap();
        fs::write(src.join("nested/a.txt"), b"hello").unwrap();
        copy_tree_replace(&src, &dest).unwrap();
        assert_eq!(
            fs::read_to_string(dest.join("nested/a.txt")).unwrap(),
            "hello"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
