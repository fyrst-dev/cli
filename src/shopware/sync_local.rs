//! `fyrst-cli shopware sync local` — VPS → local shopware-cli project-dev rsync.
//!
//! Never copies the database. Local destinations are project-tree paths
//! (`./public/media/`, `./files/`, …), never `SHOPWARE_DATA_ROOT`.

use super::env::{remote_data_root, require_shop_id, resolve_compose_dir, ShopEnv};
use super::error::Error;
use super::live::shop_basename;
use super::ssh::{resolve_ssh_source, ssh_argv_vec};
use crate::cli::SyncLocalArgs;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const DEFAULT_FROM: &str = "live";
pub const DEFAULT_SYNC_LOCAL_DATA: &str = "media,files,thumbnail,theme,sitemap";
pub const CACHE_CLEAR_REMINDER: &str = "shopware-cli project console cache:clear";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncLocalItem {
    pub logical: String,
    pub remote_src: String,
    pub local_dest: PathBuf,
    pub local_rel: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncLocalPlan {
    pub shop_root: PathBuf,
    pub from: String,
    pub ssh_target: String,
    pub ssh_cmd: Vec<String>,
    pub remote_data_root: String,
    pub shop_id: Option<String>,
    pub items: Vec<SyncLocalItem>,
    pub delete: bool,
    pub dry_run: bool,
    pub derived_root_note: Option<String>,
    pub live_checkout_warning: Option<String>,
}

impl SyncLocalPlan {
    pub fn items_csv(&self) -> String {
        self.items
            .iter()
            .map(|i| i.logical.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn mapping_line(&self, item: &SyncLocalItem) -> String {
        format!(
            "{}:{}/ → {}",
            self.ssh_target,
            item.remote_src.trim_end_matches('/'),
            item.local_rel
        )
    }

    pub fn dry_run_rsync_line(&self, item: &SyncLocalItem) -> String {
        let opts = rsync_opts(self.delete).join(" ");
        let rsh = join_quoted(&self.ssh_cmd);
        let src = format!(
            "{}:{}/",
            self.ssh_target,
            item.remote_src.trim_end_matches('/')
        );
        let dest = format!(
            "{}/",
            item.local_dest.to_string_lossy().trim_end_matches('/')
        );
        format!("rsync {opts} -e '{rsh}' {src} {dest}")
    }
}

pub fn local_project_rel(logical: &str) -> Result<&'static str, Error> {
    match logical {
        "media" => Ok("./public/media/"),
        "files" => Ok("./files/"),
        "thumbnail" => Ok("./public/thumbnail/"),
        "theme" => Ok("./public/theme/"),
        "sitemap" => Ok("./public/sitemap/"),
        _ => Err(Error::fail(format!(
            "No local project dev path for '{logical}'"
        ))),
    }
}

pub fn local_project_dest(shop_root: &Path, logical: &str) -> Result<PathBuf, Error> {
    let rel = match logical {
        "media" => "public/media",
        "files" => "files",
        "thumbnail" => "public/thumbnail",
        "theme" => "public/theme",
        "sitemap" => "public/sitemap",
        _ => {
            return Err(Error::fail(format!(
                "No local project dev path for '{logical}'"
            )))
        }
    };
    Ok(shop_root.join(rel))
}

/// rsync flags. Intentionally omits `--numeric-ids` so files are owned by the
/// local user, not VPS uid 82. `--delete` is opt-in.
pub fn rsync_opts(delete: bool) -> Vec<&'static str> {
    let mut opts = vec!["-azH"];
    if delete {
        opts.push("--delete");
    }
    opts
}

pub fn normalize_sync_local_data(spec: Option<&str>) -> Result<Vec<String>, Error> {
    let spec = spec.map(str::trim).filter(|s| !s.is_empty());
    let spec = match spec {
        None => DEFAULT_SYNC_LOCAL_DATA,
        Some(s) if s.eq_ignore_ascii_case("all") => {
            return Err(Error::fail(
                "Refusing --data all. On sync pull and backup create, all includes db. \
This command never copies the database. Omit --data (default: media,files,thumbnail,theme,sitemap) \
or list those items explicitly.",
            ));
        }
        Some(s) => s,
    };

    let mut items = Vec::new();
    let mut saw_item = false;
    for item in spec.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        saw_item = true;
        let lower = item.to_ascii_lowercase();
        match lower.as_str() {
            "media" | "files" | "thumbnail" | "theme" | "sitemap" => {
                if !items.iter().any(|v| v == &lower) {
                    items.push(lower);
                }
            }
            "all" => {
                return Err(Error::fail(
                    "Refusing --data all. On sync pull and backup create, all includes db. \
This command never copies the database. Omit --data (default: media,files,thumbnail,theme,sitemap) \
or list those items explicitly.",
                ));
            }
            "db" | "database" | "mysql" => {
                return Err(Error::fail(format!(
                    "This command does not restore the database (refusing --data '{item}'). Dump/import SQL separately (`shopware-cli project dump` + `fyrst-cli shopware db import`). VPS DB pull is `fyrst-cli shopware sync pull`. Local dest is the project dev tree, not SHOPWARE_DATA_ROOT."
                )));
            }
            "mysql_data" | "redis_data" => {
                return Err(Error::fail(format!(
                    "Refusing '{lower}'. This command only rsyncs media/files/thumbnail/theme/sitemap into the project dev tree."
                )));
            }
            _ => {
                return Err(Error::fail(format!(
                    "Unknown --data item '{item}'. Use media, files, thumbnail, theme, sitemap (not all; all includes db on sync pull / backup create)."
                )));
            }
        }
    }
    if !saw_item {
        return Err(Error::fail("--data is empty"));
    }
    if items.is_empty() {
        return Err(Error::fail("Nothing to do (--data selected an empty set)"));
    }
    Ok(items)
}

pub fn run(args: SyncLocalArgs) -> Result<(), Error> {
    let process_env: HashMap<String, String> = std::env::vars().collect();
    let cwd = std::env::current_dir().map_err(|e| Error::fail(format!("cannot read cwd: {e}")))?;
    let plan = plan(&process_env, &cwd, &args)?;
    require_cmd("rsync")?;
    require_cmd("ssh")?;
    execute(&plan)
}

pub fn plan(
    process: &HashMap<String, String>,
    cwd: &Path,
    args: &SyncLocalArgs,
) -> Result<SyncLocalPlan, Error> {
    let compose_dir = resolve_compose_dir(process, cwd)?;
    let compose_dir = fs::canonicalize(&compose_dir).unwrap_or(compose_dir);
    let env = ShopEnv::load(compose_dir, process)?;
    plan_with_env(&env, args)
}

pub fn plan_with_env(env: &ShopEnv, args: &SyncLocalArgs) -> Result<SyncLocalPlan, Error> {
    let from = match args.from.as_deref().map(str::trim) {
        None => DEFAULT_FROM.to_string(),
        Some("") => return Err(Error::fail("--from is empty")),
        Some(f) => f.to_string(),
    };

    let logicals = normalize_sync_local_data(args.data.as_deref())?;
    let (remote_root, derived_root_note) =
        resolve_remote_data_root(env, args.remote_data_root.as_deref())?;
    if remote_root.is_empty() {
        return Err(Error::fail("Remote data root is empty"));
    }

    let ssh = resolve_ssh_source(&from, env)?;
    let ssh_target = ssh.target.clone();
    let ssh_cmd = ssh_argv_vec(&ssh);
    let shop_root = env.compose_dir.clone();
    if !is_project_dev_root(&shop_root) {
        return Err(Error::fail(format!(
            "Expected a shopware-cli project root (public/ or composer.json) at {}. Run from the shop root.",
            shop_root.display()
        )));
    }

    let live_checkout_warning = live_checkout_warning(&shop_root);
    let mut items = Vec::new();
    for logical in logicals {
        let remote_src = format!("{}/{}", remote_root.trim_end_matches('/'), logical);
        let local_dest = local_project_dest(&shop_root, &logical)?;
        let local_rel = local_project_rel(&logical)?;
        refuse_vps_data_root_dest(&local_dest, env)?;
        items.push(SyncLocalItem {
            logical,
            remote_src,
            local_dest,
            local_rel,
        });
    }

    Ok(SyncLocalPlan {
        shop_root,
        from,
        ssh_target,
        ssh_cmd,
        remote_data_root: remote_root,
        shop_id: env.get("SHOPWARE_SHOP_ID").map(str::to_string),
        items,
        delete: args.delete,
        dry_run: args.dry_run,
        derived_root_note,
        live_checkout_warning,
    })
}

pub fn execute(plan: &SyncLocalPlan) -> Result<(), Error> {
    if let Some(w) = &plan.live_checkout_warning {
        eprintln!("WARNING: {w}");
    }
    if let Some(n) = &plan.derived_root_note {
        println!("==> {n}");
    }
    println!(
        "==> Live → local project dev rsync  from={}  host={}  data={}  shop={}  remote-data-root={}  delete={}  dry-run={}",
        plan.from,
        plan.ssh_target,
        plan.items_csv(),
        plan.shop_id.as_deref().unwrap_or("unset"),
        plan.remote_data_root,
        if plan.delete { 1 } else { 0 },
        if plan.dry_run { 1 } else { 0 },
    );
    println!("==> Local destinations are project-tree paths (not SHOPWARE_DATA_ROOT)");

    if plan.dry_run {
        println!(
            "==> DRY-RUN skip SSH probe {} {}",
            plan.ssh_cmd.join(" "),
            plan.ssh_target
        );
    } else {
        println!("==> Probing SSH {}", plan.ssh_target);
        probe_ssh(plan)?;
    }

    for item in &plan.items {
        println!("==> Rsync {}", plan.mapping_line(item));
        if plan.dry_run {
            println!("==> DRY-RUN {}", plan.dry_run_rsync_line(item));
            continue;
        }
        fs::create_dir_all(&item.local_dest).map_err(|e| {
            Error::fail(format!("cannot create {}: {e}", item.local_dest.display()))
        })?;
        run_rsync(plan, item)?;
    }

    if plan.dry_run {
        println!("==> DRY-RUN finished (no files copied, database not touched)");
    } else {
        println!(
            "==> Pull finished into {} (database not restored)",
            plan.shop_root.display()
        );
    }
    println!("==> Reminder: {CACHE_CLEAR_REMINDER}");
    Ok(())
}

fn live_checkout_warning(shop_root: &Path) -> Option<String> {
    let base = shop_basename(shop_root);
    if base.eq_ignore_ascii_case("live") {
        Some(
            "checkout directory is named 'live'. This command writes into local shopware-cli project dev paths (./public/media, ./files, …), not VPS SHOPWARE_DATA_ROOT. For VPS staging pull use fyrst-cli shopware sync pull — not this command.".into(),
        )
    } else {
        None
    }
}

fn is_project_dev_root(shop: &Path) -> bool {
    shop.join("public").is_dir() || shop.join("composer.json").is_file()
}

fn refuse_vps_data_root_dest(dest: &Path, env: &ShopEnv) -> Result<(), Error> {
    if let Some(root) = env.get("SHOPWARE_DATA_ROOT") {
        let root = PathBuf::from(root);
        if dest == root {
            return Err(Error::fail(format!(
                "Refusing to write into SHOPWARE_DATA_ROOT ({}); local destinations are project-tree paths (./public/media, ./files, …), never VPS bind-mount roots.",
                dest.display()
            )));
        }
    }
    Ok(())
}

fn resolve_remote_data_root(
    env: &ShopEnv,
    flag: Option<&str>,
) -> Result<(String, Option<String>), Error> {
    if let Some(flag) = flag.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok((flag.to_string(), None));
    }
    if let Some(root) = env.get("SHOPWARE_REMOTE_DATA_ROOT") {
        return Ok((root.to_string(), None));
    }
    let shop_id = require_shop_id(env).map_err(|_| {
        Error::fail(
            "SHOPWARE_SHOP_ID is required in shop-root .env (stable shop slug, same as the VPS), or pass --remote-data-root / set SHOPWARE_REMOTE_DATA_ROOT.",
        )
    })?;
    let root = remote_data_root(env, &shop_id);
    let note = format!(
        "Remote data root derived {} (shop={shop_id} env=live)",
        root.display()
    );
    Ok((root.display().to_string(), Some(note)))
}

fn posix_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-._/=:@,+%".contains(&b))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

fn join_quoted(args: &[String]) -> String {
    args.iter()
        .map(|a| posix_quote(a))
        .collect::<Vec<_>>()
        .join(" ")
}

fn require_cmd(name: &str) -> Result<(), Error> {
    if command_exists(name) {
        Ok(())
    } else {
        Err(Error::fail(format!(
            "Missing command '{name}'. Install rsync and an OpenSSH client (ssh)."
        )))
    }
}

fn command_exists(name: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg("command -v \"$1\"")
        .arg("sh")
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn probe_ssh(plan: &SyncLocalPlan) -> Result<(), Error> {
    let mut cmd = Command::new(&plan.ssh_cmd[0]);
    cmd.args(&plan.ssh_cmd[1..]);
    cmd.args(["-o", "ConnectTimeout=15", &plan.ssh_target, "true"]);
    cmd.current_dir(&plan.shop_root);
    let status = cmd
        .status()
        .map_err(|e| Error::fail(format!("failed to exec ssh: {e}")))?;
    if !status.success() {
        return Err(Error::fail(format!(
            "SSH to {} failed (BatchMode, no password prompts). Check Host {} in ~/.ssh/config, SHOPWARE_SSH_HOST/USER/KEY, and known_hosts.",
            plan.ssh_target, plan.from
        )));
    }
    Ok(())
}

fn run_rsync(plan: &SyncLocalPlan, item: &SyncLocalItem) -> Result<(), Error> {
    let rsh = join_quoted(&plan.ssh_cmd);
    let src = format!(
        "{}:{}/",
        plan.ssh_target,
        item.remote_src.trim_end_matches('/')
    );
    let dest = format!(
        "{}/",
        item.local_dest.to_string_lossy().trim_end_matches('/')
    );
    let mut cmd = Command::new("rsync");
    cmd.args(rsync_opts(plan.delete));
    cmd.arg("-e").arg(&rsh);
    cmd.arg(&src);
    cmd.arg(&dest);
    cmd.current_dir(&plan.shop_root);
    let status = cmd
        .status()
        .map_err(|e| Error::fail(format!("failed to exec rsync: {e}")))?;
    if !status.success() {
        return Err(Error::fail(format!(
            "rsync failed for {} → {}",
            src, item.local_rel
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempShop(PathBuf);
    impl TempShop {
        fn new(prefix: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let p = std::env::temp_dir().join(format!(
                "fyrst-cli-sl-{prefix}-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(p.join("deploy")).unwrap();
            fs::create_dir_all(p.join("public")).unwrap();
            fs::write(p.join("composer.json"), "{}\n").unwrap();
            Self(p)
        }
        fn named(parent_prefix: &str, basename: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let parent = std::env::temp_dir().join(format!(
                "fyrst-cli-sl-{parent_prefix}-{}-{nanos}",
                std::process::id()
            ));
            let p = parent.join(basename);
            fs::create_dir_all(p.join("deploy")).unwrap();
            fs::create_dir_all(p.join("public")).unwrap();
            fs::write(p.join("composer.json"), "{}\n").unwrap();
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
        fn write_env(&self, body: &str) {
            fs::write(self.0.join(".env"), body).unwrap();
        }
    }
    impl Drop for TempShop {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn args(
        from: Option<&str>,
        data: Option<&str>,
        remote: Option<&str>,
        delete: bool,
    ) -> SyncLocalArgs {
        SyncLocalArgs {
            from: from.map(str::to_string),
            data: data.map(str::to_string),
            remote_data_root: remote.map(str::to_string),
            delete,
            dry_run: true,
        }
    }

    fn process(shop: &Path) -> HashMap<String, String> {
        let mut p = HashMap::new();
        p.insert("COMPOSE_DIR".into(), shop.to_string_lossy().into_owned());
        p
    }

    #[test]
    fn remap_table() {
        let root = Path::new("/shop");
        for (logical, rel, dest) in [
            ("media", "./public/media/", "/shop/public/media"),
            ("files", "./files/", "/shop/files"),
            ("thumbnail", "./public/thumbnail/", "/shop/public/thumbnail"),
            ("theme", "./public/theme/", "/shop/public/theme"),
            ("sitemap", "./public/sitemap/", "/shop/public/sitemap"),
        ] {
            assert_eq!(local_project_rel(logical).unwrap(), rel);
            assert_eq!(
                local_project_dest(root, logical).unwrap(),
                PathBuf::from(dest)
            );
        }
    }

    #[test]
    fn default_omitted_data_is_not_db() {
        let d = normalize_sync_local_data(None).unwrap();
        assert_eq!(d, vec!["media", "files", "thumbnail", "theme", "sitemap"]);
        assert!(!d.iter().any(|i| i == "db"));
    }

    #[test]
    fn refuse_data_all() {
        for spec in ["all", "ALL", "All"] {
            let err = normalize_sync_local_data(Some(spec)).unwrap_err();
            let m = err.to_string();
            assert!(m.contains("Refusing --data all"), "{m}");
            assert!(m.contains("includes db"), "{m}");
        }
        let err = normalize_sync_local_data(Some("media,all")).unwrap_err();
        assert!(err.to_string().contains("Refusing --data all"), "{err}");
    }

    #[test]
    fn refuse_db_aliases() {
        for spec in ["db", "database", "mysql", "DB"] {
            let err = normalize_sync_local_data(Some(spec)).unwrap_err();
            let m = err.to_string();
            assert!(m.contains("does not restore the database"), "{m}");
            assert!(m.contains("shopware-cli project dump"), "{m}");
            assert!(m.contains("db import"), "{m}");
            assert!(m.contains("SHOPWARE_DATA_ROOT"), "{m}");
        }
    }

    #[test]
    fn refuse_named_volumes() {
        for spec in ["mysql_data", "redis_data"] {
            let err = normalize_sync_local_data(Some(spec)).unwrap_err();
            let m = err.to_string();
            assert!(m.contains(spec), "{m}");
            assert!(m.contains("project dev tree"), "{m}");
        }
    }

    #[test]
    fn unknown_item_fails() {
        let err = normalize_sync_local_data(Some("logs")).unwrap_err();
        assert!(err.to_string().contains("Unknown"), "{err}");
    }

    #[test]
    fn rsync_opts_omit_numeric_ids_delete_opt_in() {
        let off = rsync_opts(false);
        assert_eq!(off, vec!["-azH"]);
        assert!(!off.contains(&"--delete"));
        assert!(!off.contains(&"--numeric-ids"));
        let on = rsync_opts(true);
        assert_eq!(on, vec!["-azH", "--delete"]);
        assert!(!on.contains(&"--numeric-ids"));
    }

    #[test]
    fn dry_run_plan_default_from_live_and_remap() {
        let shop = TempShop::new("plan");
        shop.write_env(
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=dev
SHOPWARE_DATA_ROOT=/var/lib/shopware/data/acme/dev
",
        );
        let plan = plan(
            &process(shop.path()),
            shop.path(),
            &args(None, None, None, false),
        )
        .unwrap();
        assert_eq!(plan.from, "live");
        assert_eq!(plan.ssh_target, "live");
        assert_eq!(plan.remote_data_root, "/var/lib/shopware/data/acme/live");
        assert!(plan.derived_root_note.is_some());
        assert!(!plan.delete);
        assert!(plan.dry_run);
        assert_eq!(
            plan.ssh_cmd,
            ["ssh", "-o", "BatchMode=yes", "-p", "22"]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>()
        );
        let rels: Vec<_> = plan.items.iter().map(|i| i.local_rel).collect();
        assert_eq!(
            rels,
            [
                "./public/media/",
                "./files/",
                "./public/thumbnail/",
                "./public/theme/",
                "./public/sitemap/"
            ]
        );
        for item in &plan.items {
            assert!(
                item.local_dest.starts_with(shop.path()),
                "{}",
                item.local_dest.display()
            );
            assert_ne!(
                item.local_dest,
                PathBuf::from("/var/lib/shopware/data/acme/dev")
            );
            assert!(!item.local_dest.starts_with("/var/lib/shopware/data"));
            let line = plan.mapping_line(item);
            assert!(
                line.starts_with("live:/var/lib/shopware/data/acme/live/"),
                "{line}"
            );
            assert!(line.contains(" → "), "{line}");
            assert!(line.contains(item.local_rel), "{line}");
            let dry = plan.dry_run_rsync_line(item);
            assert!(
                dry.starts_with("rsync -azH -e 'ssh -o BatchMode=yes -p 22'"),
                "{dry}"
            );
            assert!(!dry.contains("--delete"), "{dry}");
            assert!(!dry.contains("--numeric-ids"), "{dry}");
            assert!(
                dry.contains(&item.local_dest.to_string_lossy().into_owned()),
                "{dry}"
            );
        }
        let media = plan.items.iter().find(|i| i.logical == "media").unwrap();
        assert_eq!(
            plan.mapping_line(media),
            "live:/var/lib/shopware/data/acme/live/media/ → ./public/media/"
        );
        let files = plan.items.iter().find(|i| i.logical == "files").unwrap();
        assert_eq!(
            plan.mapping_line(files),
            "live:/var/lib/shopware/data/acme/live/files/ → ./files/"
        );
    }

    #[test]
    fn missing_shop_id_without_remote_root_fails() {
        let shop = TempShop::new("noid");
        shop.write_env("SHOPWARE_DEPLOY_ENV=dev\n");
        let err = plan(
            &process(shop.path()),
            shop.path(),
            &args(None, Some("media"), None, false),
        )
        .unwrap_err();
        let m = err.to_string();
        assert!(m.contains("SHOPWARE_SHOP_ID"), "{m}");
        assert!(m.contains("--remote-data-root"), "{m}");
    }

    #[test]
    fn remote_data_root_flag_skips_shop_id() {
        let shop = TempShop::new("flag");
        shop.write_env("SHOPWARE_DEPLOY_ENV=dev\n");
        let plan = plan(
            &process(shop.path()),
            shop.path(),
            &args(None, Some("media"), Some("/mnt/uploads"), false),
        )
        .unwrap();
        assert_eq!(plan.remote_data_root, "/mnt/uploads");
        assert!(plan.shop_id.is_none());
        assert_eq!(
            plan.mapping_line(&plan.items[0]),
            "live:/mnt/uploads/media/ → ./public/media/"
        );
    }

    #[test]
    fn shopware_ssh_and_remote_root() {
        let shop = TempShop::new("alias");
        shop.write_env(
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_REMOTE_DATA_ROOT=/vps/staging-data
SHOPWARE_SSH_HOST=staging.example
SHOPWARE_SSH_USER=deploy
",
        );
        let plan = plan(
            &process(shop.path()),
            shop.path(),
            &args(Some("staging"), Some("theme"), None, true),
        )
        .unwrap();
        assert_eq!(plan.from, "staging");
        assert_eq!(plan.ssh_target, "deploy@staging.example");
        assert_eq!(plan.remote_data_root, "/vps/staging-data");
        assert!(plan.delete);
        assert!(plan
            .ssh_cmd
            .windows(2)
            .any(|w| w[0] == "-p" && w[1] == "22"));
        let dry = plan.dry_run_rsync_line(&plan.items[0]);
        assert!(dry.contains("--delete"), "{dry}");
        assert_eq!(
            plan.mapping_line(&plan.items[0]),
            "deploy@staging.example:/vps/staging-data/theme/ → ./public/theme/"
        );
    }

    #[test]
    fn process_identity_wins_for_remote_root() {
        let shop = TempShop::new("preset");
        shop.write_env(
            "\
SHOPWARE_SHOP_ID=fromfile
SHOPWARE_REMOTE_DATA_ROOT=/fromfile
",
        );
        let mut process = process(shop.path());
        process.insert("SHOPWARE_SHOP_ID".into(), "fromproc".into());
        process.insert("SHOPWARE_REMOTE_DATA_ROOT".into(), "/fromproc".into());
        let plan = plan(
            &process,
            shop.path(),
            &args(None, Some("media"), None, false),
        )
        .unwrap();
        assert_eq!(plan.shop_id.as_deref(), Some("fromproc"));
        assert_eq!(plan.remote_data_root, "/fromproc");
    }

    #[test]
    fn env_prod_is_loaded() {
        let shop = TempShop::new("prod");
        shop.write_env("SHOPWARE_SHOP_ID=fromenv\n");
        fs::write(
            shop.path().join(".env.prod"),
            "SHOPWARE_SHOP_ID=fromprod\nSHOPWARE_DATA_BASE=/fromprod\n",
        )
        .unwrap();
        let plan = plan(
            &process(shop.path()),
            shop.path(),
            &args(None, Some("media"), None, false),
        )
        .unwrap();
        assert_eq!(plan.shop_id.as_deref(), Some("fromprod"));
        assert_eq!(plan.remote_data_root, "/fromprod/fromprod/live");
    }

    #[test]
    fn env_local_sets_ssh_host() {
        let shop = TempShop::new("syncenv");
        shop.write_env("SHOPWARE_SHOP_ID=acme\n");
        fs::write(
            shop.path().join(".env.local"),
            "SHOPWARE_SSH_HOST=vps.example\nSHOPWARE_SSH_USER=root\n",
        )
        .unwrap();
        let plan = plan(
            &process(shop.path()),
            shop.path(),
            &args(None, Some("files"), None, false),
        )
        .unwrap();
        assert_eq!(plan.ssh_target, "root@vps.example");
        assert_eq!(
            plan.mapping_line(&plan.items[0]),
            "root@vps.example:/var/lib/shopware/data/acme/live/files/ → ./files/"
        );
    }

    #[test]
    fn live_checkout_warns() {
        let shop = TempShop::named("ck", "live");
        shop.write_env("SHOPWARE_SHOP_ID=acme\n");
        let plan = plan(
            &process(shop.path()),
            shop.path(),
            &args(None, Some("media"), None, false),
        )
        .unwrap();
        let w = plan.live_checkout_warning.expect("warning");
        assert!(w.contains("named 'live'"), "{w}");
        assert!(w.contains("SHOPWARE_DATA_ROOT"), "{w}");
    }

    #[test]
    fn missing_ssh_key_fails() {
        let shop = TempShop::new("key");
        shop.write_env(
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_SSH_KEY=/no/such/key
",
        );
        let err = plan(
            &process(shop.path()),
            shop.path(),
            &args(None, Some("media"), None, false),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("SHOPWARE_SSH_KEY not found"),
            "{err}"
        );
    }

    #[test]
    fn missing_project_markers_fail() {
        let shop = TempShop::new("noproj");
        shop.write_env("SHOPWARE_SHOP_ID=acme\n");
        fs::remove_dir_all(shop.path().join("public")).unwrap();
        fs::remove_file(shop.path().join("composer.json")).unwrap();
        let err = plan(
            &process(shop.path()),
            shop.path(),
            &args(None, Some("media"), None, false),
        )
        .unwrap_err();
        assert!(err.to_string().contains("composer.json"), "{err}");
    }

    #[test]
    fn leftover_source_env_fails_without_remote_root() {
        let shop = TempShop::new("srcenv");
        shop.write_env(
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DATA_BASE=/data
SYNC_SOURCE_ENV=staging
",
        );
        let err = plan(
            &process(shop.path()),
            shop.path(),
            &args(None, Some("sitemap"), None, false),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("SHOPWARE_REMOTE_DATA_ROOT"),
            "{err}"
        );
    }
}
