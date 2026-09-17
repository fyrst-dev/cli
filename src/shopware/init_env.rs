//! `fyrst-cli shopware env init` — finish shop-root `.env` after create + Flex.
//!
//! Shared `.env` is git-committed and must not hold env-specific values.
//! This command may set `SHOPWARE_SHOP_ID` (same slug everywhere) and optional
//! `IMAGE`, merge missing keys from `.env.example`, and **comments out**
//! uncommented `COMPOSE_PROJECT_NAME` (create writes `sw-…`) and leftover
//! `SHOPWARE_DEPLOY_ENV` in shared `.env`. Deploy env and
//! `COMPOSE_PROJECT_NAME=<shop-id>-<env>` are written to `.env.local`
//! (gitignored). Top-level Compose `name: <shop-id>-<env>` is upserted in
//! gitignored `compose.override.yaml` so `shopware-cli project dev` uses that
//! name (Compose auto-reads `COMPOSE_PROJECT_NAME` only from project-directory
//! `.env`, not `.env.local`). VPS Compose project is the same
//! `{SHOPWARE_SHOP_ID}-{SHOPWARE_DEPLOY_ENV}` via identity + `docker compose -p`.
//! Does not invent MYSQL passwords or `APP_URL`, and does not generate or
//! rewrite `APP_SECRET` (`shopware-cli project create` writes that). Never
//! prints secrets. This is not a dump command.

use super::compose_override::{upsert_top_level_name, OVERRIDE_REL};
use super::env::{resolve_compose_dir_init, vps_project_name};
use super::envfile::{
    comment_compose_project_name, comment_deploy_env, last_value, merge_missing_from_example,
    set_key,
};
use super::error::Error;
use crate::cli::InitEnvArgs;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) struct Plan {
    pub compose_dir: PathBuf,
    pub env_file: PathBuf,
    pub local_env_file: PathBuf,
    pub override_file: PathBuf,
    contents: String,
    local_contents: String,
    override_contents: String,
    pub dry_run: bool,
    pub copied: bool,
    pub merged_keys: Vec<String>,
    pub shop_id: String,
    pub shop_id_changed: bool,
    pub deploy_env: String,
    pub deploy_env_changed: bool,
    pub local_created: bool,
    pub compose_project_name: String,
    pub compose_project_name_changed: bool,
    pub override_created: bool,
    pub override_changed: bool,
    pub image_set: Option<String>,
    pub compose_project_name_commented: usize,
    pub deploy_env_commented: usize,
}

impl std::fmt::Debug for Plan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plan")
            .field("compose_dir", &self.compose_dir)
            .field("env_file", &self.env_file)
            .field("local_env_file", &self.local_env_file)
            .field("override_file", &self.override_file)
            .field("dry_run", &self.dry_run)
            .field("copied", &self.copied)
            .field("merged_keys", &self.merged_keys)
            .field("shop_id", &self.shop_id)
            .field("shop_id_changed", &self.shop_id_changed)
            .field("deploy_env", &self.deploy_env)
            .field("deploy_env_changed", &self.deploy_env_changed)
            .field("local_created", &self.local_created)
            .field("compose_project_name", &self.compose_project_name)
            .field(
                "compose_project_name_changed",
                &self.compose_project_name_changed,
            )
            .field("override_created", &self.override_created)
            .field("override_changed", &self.override_changed)
            .field("image_set", &self.image_set)
            .field(
                "compose_project_name_commented",
                &self.compose_project_name_commented,
            )
            .field("deploy_env_commented", &self.deploy_env_commented)
            .finish_non_exhaustive()
    }
}

impl Plan {
    #[cfg(test)]
    pub(crate) fn contents(&self) -> &str {
        &self.contents
    }

    #[cfg(test)]
    pub(crate) fn local_contents(&self) -> &str {
        &self.local_contents
    }

    #[cfg(test)]
    pub(crate) fn override_contents(&self) -> &str {
        &self.override_contents
    }
}

pub fn run(args: InitEnvArgs) -> Result<(), Error> {
    refuse_xtrace()?;
    let process_env: HashMap<String, String> = std::env::vars().collect();
    let cwd = std::env::current_dir().map_err(|e| Error::fail(format!("cannot read cwd: {e}")))?;
    let plan = plan(&process_env, &cwd, &args)?;
    execute(&plan)
}

fn refuse_xtrace() -> Result<(), Error> {
    if let Ok(opts) = std::env::var("SHELLOPTS") {
        if opts.split(':').any(|o| o == "xtrace") {
            return Err(Error::fail(
                "refusing to run with xtrace (credentials may be in .env)",
            ));
        }
    }
    if std::env::var_os("BASH_XTRACEFD").is_some() {
        return Err(Error::fail(
            "refusing to run with xtrace (credentials may be in .env)",
        ));
    }
    Ok(())
}

pub(crate) fn plan(
    process_env: &HashMap<String, String>,
    cwd: &Path,
    args: &InitEnvArgs,
) -> Result<Plan, Error> {
    let compose_dir = resolve_compose_dir_init(process_env, cwd)?;
    let env_file = compose_dir.join(".env");
    let local_env_file = compose_dir.join(".env.local");
    let override_file = compose_dir.join(OVERRIDE_REL);
    let example_file = compose_dir.join(".env.example");

    let env_exists = env_file.is_file();
    let example_exists = example_file.is_file();
    if !env_exists && !example_exists {
        return Err(Error::fail(format!(
            "Missing {} and {}. Run this from the shop root after Flex copied .env.example (composer require fyrst/shopware-cd), or copy a shop .env into COMPOSE_DIR={}.",
            env_file.display(),
            example_file.display(),
            compose_dir.display()
        )));
    }

    let copied = !env_exists;
    let example_contents = if example_exists {
        Some(read_file(&example_file)?)
    } else {
        None
    };
    let mut contents = if env_exists {
        read_file(&env_file)?
    } else {
        example_contents.as_deref().unwrap_or("").to_string()
    };

    let mut merged_keys = Vec::new();
    if let Some(example) = &example_contents {
        merged_keys = merge_missing_from_example(example, &mut contents);
    }

    let local_exists = local_env_file.is_file();
    let mut local_contents = if local_exists {
        read_file(&local_env_file)?
    } else {
        String::new()
    };
    let prod_deploy_env = read_optional_key(&compose_dir.join(".env.prod"), "SHOPWARE_DEPLOY_ENV")?;

    let existing_shop_id = last_value(&contents, "SHOPWARE_SHOP_ID");
    let leftover_shared_deploy_env = last_value(&contents, "SHOPWARE_DEPLOY_ENV");
    let existing_host_deploy_env = last_value(&local_contents, "SHOPWARE_DEPLOY_ENV");
    let existing_host_compose_project_name = last_value(&local_contents, "COMPOSE_PROJECT_NAME");
    let existing_image = last_value(&contents, "IMAGE");
    let override_exists = override_file.is_file();
    let existing_override = if override_exists {
        read_file(&override_file)?
    } else {
        String::new()
    };

    let shop_id_flag = nonempty_flag(args.shop_id.as_deref(), "--shop-id")?;
    let shop_id = match shop_id_flag {
        Some(s) => s.to_string(),
        None => existing_shop_id.clone(),
    };
    if shop_id.is_empty() {
        return Err(Error::fail(
            "SHOPWARE_SHOP_ID is empty. Pass --shop-id (same slug on live, staging, and laptop).",
        ));
    }
    validate_shop_id(&shop_id)?;

    let deploy_env = match args.env {
        Some(e) => e.as_str().to_string(),
        None if !existing_host_deploy_env.is_empty() => existing_host_deploy_env.clone(),
        None if !prod_deploy_env.is_empty() => prod_deploy_env,
        None if !leftover_shared_deploy_env.is_empty() => leftover_shared_deploy_env.clone(),
        None => "live".to_string(),
    };
    validate_deploy_env(&deploy_env)?;

    let image_flag = nonempty_flag(args.image.as_deref(), "--image")?;
    if let Some(image) = image_flag {
        if image.chars().any(char::is_whitespace) {
            return Err(Error::fail(format!(
                "Invalid --image '{image}' (no whitespace)."
            )));
        }
    }

    contents = set_key(&contents, "SHOPWARE_SHOP_ID", &shop_id);
    let mut image_set = None;
    if let Some(image) = image_flag {
        if existing_image != image {
            image_set = Some(image.to_string());
        }
        contents = set_key(&contents, "IMAGE", image);
    }

    let (contents, compose_project_name_commented) = comment_compose_project_name(&contents);
    let (contents, deploy_env_commented) = comment_deploy_env(&contents);

    let compose_project_name = vps_project_name(&shop_id, &deploy_env);
    local_contents = set_key(&local_contents, "SHOPWARE_DEPLOY_ENV", &deploy_env);
    local_contents = set_key(
        &local_contents,
        "COMPOSE_PROJECT_NAME",
        &compose_project_name,
    );
    let (override_contents, override_changed) =
        upsert_top_level_name(&existing_override, &compose_project_name);

    let shop_id_changed = existing_shop_id != shop_id;
    let deploy_env_changed = existing_host_deploy_env != deploy_env;
    let compose_project_name_changed = existing_host_compose_project_name != compose_project_name;
    Ok(Plan {
        compose_dir,
        env_file,
        local_env_file,
        override_file,
        contents,
        local_contents,
        override_contents,
        dry_run: args.dry_run,
        copied,
        merged_keys,
        shop_id,
        shop_id_changed,
        deploy_env,
        deploy_env_changed,
        local_created: !local_exists,
        compose_project_name,
        compose_project_name_changed,
        override_created: !override_exists,
        override_changed,
        image_set,
        compose_project_name_commented,
        deploy_env_commented,
    })
}

fn read_optional_key(path: &Path, key: &str) -> Result<String, Error> {
    if !path.is_file() {
        return Ok(String::new());
    }
    Ok(last_value(&read_file(path)?, key))
}

fn nonempty_flag<'a>(value: Option<&'a str>, flag: &str) -> Result<Option<&'a str>, Error> {
    match value {
        None => Ok(None),
        Some(s) if s.is_empty() => Err(Error::fail(format!("{flag} requires a value"))),
        Some(s) => Ok(Some(s)),
    }
}

fn read_file(path: &Path) -> Result<String, Error> {
    fs::read_to_string(path)
        .map_err(|e| Error::fail(format!("cannot read {}: {e}", path.display())))
}

fn validate_shop_id(slug: &str) -> Result<(), Error> {
    if is_shop_slug(slug) {
        Ok(())
    } else {
        Err(Error::fail(format!(
            "Invalid --shop-id '{slug}'. Use a lowercase slug (letters, digits, hyphens), e.g. acme."
        )))
    }
}

fn is_shop_slug(slug: &str) -> bool {
    let b = slug.as_bytes();
    if b.is_empty() {
        return false;
    }
    let alnum = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    if !alnum(b[0]) || !alnum(b[b.len() - 1]) {
        return false;
    }
    b.iter().all(|c| alnum(*c) || *c == b'-')
}

fn validate_deploy_env(name: &str) -> Result<(), Error> {
    match name {
        "live" | "staging" | "playground" | "dev" => Ok(()),
        _ => Err(Error::fail(format!(
            "Invalid --env '{name}'. Use live, staging, playground, or dev."
        ))),
    }
}

fn execute(plan: &Plan) -> Result<(), Error> {
    if !plan.dry_run {
        fs::write(&plan.env_file, &plan.contents)
            .map_err(|e| Error::fail(format!("cannot write {}: {e}", plan.env_file.display())))?;
        chmod_600(&plan.env_file)?;
        fs::write(&plan.local_env_file, &plan.local_contents).map_err(|e| {
            Error::fail(format!(
                "cannot write {}: {e}",
                plan.local_env_file.display()
            ))
        })?;
        chmod_600(&plan.local_env_file)?;
        fs::write(&plan.override_file, &plan.override_contents).map_err(|e| {
            Error::fail(format!(
                "cannot write {}: {e}",
                plan.override_file.display()
            ))
        })?;
    }
    print!("{}", summary_text(plan));
    Ok(())
}

pub(crate) fn summary_text(plan: &Plan) -> String {
    let mut s = String::new();
    if plan.dry_run {
        let _ = writeln!(
            s,
            "==> DRY-RUN (no write) COMPOSE_DIR={}",
            plan.compose_dir.display()
        );
    } else {
        let _ = writeln!(s, "==> Updated {}", plan.env_file.display());
        let _ = writeln!(s, "==> Updated {}", plan.local_env_file.display());
        let _ = writeln!(s, "==> Updated {}", plan.override_file.display());
    }

    let mut changes = 0u32;
    if plan.copied {
        let _ = writeln!(s, "  copy .env.example → .env");
        changes += 1;
    }
    if !plan.merged_keys.is_empty() {
        let _ = writeln!(
            s,
            "  merge missing keys from .env.example: {}",
            plan.merged_keys.join(", ")
        );
        changes += 1;
    }
    if plan.shop_id_changed {
        let _ = writeln!(s, "  set SHOPWARE_SHOP_ID={}", plan.shop_id);
        changes += 1;
    }
    if let Some(image) = &plan.image_set {
        let _ = writeln!(s, "  set IMAGE={image}");
        changes += 1;
    }
    if plan.compose_project_name_commented > 0 {
        let _ = writeln!(
            s,
            "  commented {} COMPOSE_PROJECT_NAME=… line(s) in shared .env",
            plan.compose_project_name_commented
        );
        changes += 1;
    } else {
        let _ = writeln!(s, "  no uncommented COMPOSE_PROJECT_NAME=… in shared .env");
    }
    if plan.deploy_env_commented > 0 {
        let _ = writeln!(
            s,
            "  commented {} SHOPWARE_DEPLOY_ENV=… line(s) in shared .env",
            plan.deploy_env_commented
        );
        changes += 1;
    } else {
        let _ = writeln!(s, "  no uncommented SHOPWARE_DEPLOY_ENV=… in shared .env");
    }
    if plan.local_created {
        let _ = writeln!(s, "  create .env.local");
        changes += 1;
    }
    if plan.deploy_env_changed {
        let _ = writeln!(
            s,
            "  set SHOPWARE_DEPLOY_ENV={} in .env.local",
            plan.deploy_env
        );
        changes += 1;
    } else {
        let _ = writeln!(
            s,
            "  already set SHOPWARE_DEPLOY_ENV={} in .env.local",
            plan.deploy_env
        );
    }
    if plan.compose_project_name_changed {
        let _ = writeln!(
            s,
            "  set COMPOSE_PROJECT_NAME={} in .env.local",
            plan.compose_project_name
        );
        changes += 1;
    } else {
        let _ = writeln!(
            s,
            "  already set COMPOSE_PROJECT_NAME={} in .env.local",
            plan.compose_project_name
        );
    }
    if plan.override_created {
        let _ = writeln!(s, "  create {OVERRIDE_REL}");
        changes += 1;
    }
    if plan.override_changed {
        let _ = writeln!(
            s,
            "  set name: {} in {OVERRIDE_REL}",
            plan.compose_project_name
        );
        changes += 1;
    } else {
        let _ = writeln!(
            s,
            "  already set name: {} in {OVERRIDE_REL}",
            plan.compose_project_name
        );
    }
    if changes == 0 {
        let _ = writeln!(s, "  no changes (already up to date)");
    }
    let _ = writeln!(
        s,
        "  left unchanged: MYSQL passwords, APP_URL, APP_SECRET (fill MYSQL/APP_URL by hand)"
    );
    if !plan.dry_run {
        let _ = writeln!(s, "==> chmod 600 .env");
        let _ = writeln!(s, "==> chmod 600 .env.local");
    }
    s
}

#[cfg(unix)]
fn chmod_600(path: &Path) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|e| Error::fail(format!("cannot chmod 600 {}: {e}", path.display())))
}

#[cfg(not(unix))]
fn chmod_600(_path: &Path) -> Result<(), Error> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::envfile::{has_key, MERGE_FROM_EXAMPLE_HEADER};
    use super::*;
    use crate::cli::DeployEnv;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempShop(PathBuf);

    impl TempShop {
        fn new(prefix: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let p = std::env::temp_dir().join(format!(
                "fyrst-cli-init-env-{prefix}-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(p.join("deploy")).unwrap();
            Self(p)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn write_env(&self, s: &str) {
            fs::write(self.0.join(".env"), s).unwrap();
        }

        fn write_local(&self, s: &str) {
            fs::write(self.0.join(".env.local"), s).unwrap();
        }

        fn write_override(&self, s: &str) {
            fs::write(self.0.join(OVERRIDE_REL), s).unwrap();
        }

        fn write_example(&self, s: &str) {
            fs::write(self.0.join(".env.example"), s).unwrap();
        }
    }

    impl Drop for TempShop {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn process(shop: &Path) -> HashMap<String, String> {
        let mut p = HashMap::new();
        p.insert("COMPOSE_DIR".into(), shop.to_string_lossy().into_owned());
        p
    }

    fn args() -> InitEnvArgs {
        InitEnvArgs {
            shop_id: Some("acme".into()),
            env: None,
            image: None,
            dry_run: true,
        }
    }

    const EXISTING_SECRET: &str = "existing-app-secret-do-not-print-0123456789abcdef";
    const MYSQL_PASS: &str = "super-secret-pass";

    fn assert_no_uncommented(contents: &str, key: &str) {
        assert!(
            !has_key(contents, key),
            "shared .env still has uncommented {key}:\n{contents}"
        );
    }

    #[test]
    fn dry_run_summary_hides_passwords_and_does_not_write() {
        let shop = TempShop::new("dry");
        shop.write_env(&format!(
            "\
SHOPWARE_SHOP_ID=
SHOPWARE_DEPLOY_ENV=
MYSQL_PASSWORD={MYSQL_PASS}
APP_SECRET={EXISTING_SECRET}
COMPOSE_PROJECT_NAME=sw-shop-acme
"
        ));
        let mut a = args();
        a.image = Some("ghcr.io/example/acme".into());
        let p = plan(&process(shop.path()), shop.path(), &a).unwrap();
        assert!(p.dry_run);
        assert_eq!(p.shop_id, "acme");
        assert_eq!(p.deploy_env, "live");
        assert!(p.local_created);
        assert_eq!(p.image_set.as_deref(), Some("ghcr.io/example/acme"));
        assert!(p.compose_project_name_commented >= 1);
        assert!(p.deploy_env_commented >= 1);
        assert_eq!(last_value(p.contents(), "APP_SECRET"), EXISTING_SECRET);
        assert_no_uncommented(p.contents(), "COMPOSE_PROJECT_NAME");
        assert_no_uncommented(p.contents(), "SHOPWARE_DEPLOY_ENV");
        assert_eq!(
            last_value(p.local_contents(), "SHOPWARE_DEPLOY_ENV"),
            "live"
        );
        assert_eq!(
            last_value(p.local_contents(), "COMPOSE_PROJECT_NAME"),
            "acme-live"
        );
        assert_eq!(p.compose_project_name, "acme-live");
        assert!(p.override_created);
        assert!(p.override_changed);
        assert!(p.override_contents().contains("name: acme-live\n"));
        assert!(!p.contents().contains("COMPOSE_PROJECT_NAME=shopware-"));
        assert!(!p.contents().contains("COMPOSE_PROJECT_NAME=acme-"));
        let text = summary_text(&p);
        assert!(text.contains("DRY-RUN (no write)"));
        assert!(text.contains("set SHOPWARE_SHOP_ID=acme"));
        assert!(text.contains("set SHOPWARE_DEPLOY_ENV=live in .env.local"));
        assert!(text.contains("set COMPOSE_PROJECT_NAME=acme-live in .env.local"));
        assert!(text.contains("create compose.override.yaml"));
        assert!(text.contains("set name: acme-live in compose.override.yaml"));
        assert!(text.contains("commented "));
        assert!(text.contains("COMPOSE_PROJECT_NAME"));
        assert!(text.contains("create .env.local"));
        assert!(text.contains("set IMAGE=ghcr.io/example/acme"));
        assert!(!text.contains("--vps"));
        assert!(text.contains("APP_SECRET"));
        assert!(!text.contains("generate-app-secret"));
        assert!(!text.contains(MYSQL_PASS));
        assert!(!text.contains(EXISTING_SECRET));
        let after = fs::read_to_string(shop.path().join(".env")).unwrap();
        assert!(
            after.contains("COMPOSE_PROJECT_NAME=sw-shop-acme"),
            "dry-run must not write .env"
        );
        assert!(!shop.path().join(".env.local").is_file());
        assert!(!shop.path().join(OVERRIDE_REL).is_file());
    }

    #[test]
    fn write_path_sets_shop_id_comments_shared_keys_and_writes_local() {
        let shop = TempShop::new("write");
        shop.write_env(&format!(
            "\
SHOPWARE_SHOP_ID=
MYSQL_PASSWORD={MYSQL_PASS}
APP_URL=
COMPOSE_PROJECT_NAME=sw-shop-acme
SHOPWARE_DEPLOY_ENV=live
"
        ));
        shop.write_example(
            "\
SHOPWARE_SHOP_ID=
MYSQL_PASSWORD=from-example
APP_URL=http://localhost
NEW_FROM_EXAMPLE=1
COMPOSE_PROJECT_NAME=from-example
SHOPWARE_DEPLOY_ENV=from-example
",
        );
        let mut a = args();
        a.dry_run = false;
        a.env = Some(DeployEnv::Staging);
        let p = plan(&process(shop.path()), shop.path(), &a).unwrap();
        assert!(p.merged_keys.contains(&"NEW_FROM_EXAMPLE".into()));
        assert!(!p.merged_keys.contains(&"MYSQL_PASSWORD".into()));
        assert!(!p.merged_keys.contains(&"COMPOSE_PROJECT_NAME".into()));
        assert!(!p.merged_keys.contains(&"SHOPWARE_DEPLOY_ENV".into()));
        assert_eq!(last_value(p.contents(), "SHOPWARE_SHOP_ID"), "acme");
        assert_no_uncommented(p.contents(), "SHOPWARE_DEPLOY_ENV");
        assert_eq!(last_value(p.contents(), "MYSQL_PASSWORD"), MYSQL_PASS);
        assert!(p.contents().contains(MERGE_FROM_EXAMPLE_HEADER));
        assert!(p.contents().contains("NEW_FROM_EXAMPLE=1"));
        assert_no_uncommented(p.contents(), "COMPOSE_PROJECT_NAME");
        assert!(p
            .contents()
            .contains("# COMPOSE_PROJECT_NAME=sw-shop-acme # commented by fyrst-cli"));
        assert_eq!(
            last_value(p.local_contents(), "SHOPWARE_DEPLOY_ENV"),
            "staging"
        );
        assert_eq!(
            last_value(p.local_contents(), "COMPOSE_PROJECT_NAME"),
            "acme-staging"
        );
        assert!(p.override_contents().contains("name: acme-staging\n"));
        assert!(!p.contents().contains("COMPOSE_PROJECT_NAME=shopware-"));
        assert!(!p.contents().contains("COMPOSE_PROJECT_NAME=acme-"));
        assert!(!p.contents().contains("--vps"));
        let text = summary_text(&p);
        assert!(!text.contains(MYSQL_PASS));
        assert!(!text.contains("--vps"));
        assert!(text.contains("merge missing keys from .env.example: NEW_FROM_EXAMPLE"));
        assert!(text.contains("set SHOPWARE_DEPLOY_ENV=staging in .env.local"));
        assert!(text.contains("set COMPOSE_PROJECT_NAME=acme-staging in .env.local"));
        assert!(text.contains("set name: acme-staging in compose.override.yaml"));
        assert!(text.contains("commented "));
    }

    #[test]
    fn already_commented_compose_project_name_is_idempotent() {
        let shop = TempShop::new("already-commented");
        shop.write_env(
            "\
SHOPWARE_SHOP_ID=acme
# COMPOSE_PROJECT_NAME=sw-shop-acme
",
        );
        shop.write_local("SHOPWARE_DEPLOY_ENV=live\nCOMPOSE_PROJECT_NAME=acme-live\n");
        shop.write_override("name: acme-live\n");
        let mut a = args();
        a.shop_id = None;
        let p = plan(&process(shop.path()), shop.path(), &a).unwrap();
        assert_eq!(p.compose_project_name_commented, 0);
        assert!(!p.deploy_env_changed);
        assert!(!p.compose_project_name_changed);
        assert!(!p.override_changed);
        assert!(!p.override_created);
        assert!(!p.local_created);
        assert!(p.contents().contains("# COMPOSE_PROJECT_NAME=sw-shop-acme"));
        assert_no_uncommented(p.contents(), "COMPOSE_PROJECT_NAME");
        assert_eq!(
            last_value(p.local_contents(), "SHOPWARE_DEPLOY_ENV"),
            "live"
        );
        assert_eq!(
            last_value(p.local_contents(), "COMPOSE_PROJECT_NAME"),
            "acme-live"
        );
        let text = summary_text(&p);
        assert!(text.contains("no uncommented COMPOSE_PROJECT_NAME=… in shared .env"));
        assert!(text.contains("already set SHOPWARE_DEPLOY_ENV=live in .env.local"));
        assert!(text.contains("already set COMPOSE_PROJECT_NAME=acme-live in .env.local"));
        assert!(text.contains("already set name: acme-live in compose.override.yaml"));
        assert!(!text.contains("--vps"));
        assert!(text.contains("no changes (already up to date)"));
    }

    #[test]
    fn copy_from_example_comments_compose_project_name_and_writes_local() {
        let shop = TempShop::new("copy-cpn");
        shop.write_example(
            "\
SHOPWARE_SHOP_ID=
COMPOSE_PROJECT_NAME=sw-shop-acme
SHOPWARE_DEPLOY_ENV=live
MYSQL_PASSWORD=example-secret
",
        );
        let p = plan(&process(shop.path()), shop.path(), &args()).unwrap();
        assert!(p.copied);
        assert!(p.compose_project_name_commented >= 1);
        assert_no_uncommented(p.contents(), "COMPOSE_PROJECT_NAME");
        assert_no_uncommented(p.contents(), "SHOPWARE_DEPLOY_ENV");
        assert!(!p.contents().contains("COMPOSE_PROJECT_NAME=sw-shop-acme\n"));
        assert_eq!(
            last_value(p.local_contents(), "SHOPWARE_DEPLOY_ENV"),
            "live"
        );
        assert_eq!(
            last_value(p.local_contents(), "COMPOSE_PROJECT_NAME"),
            "acme-live"
        );
        assert!(p.override_contents().contains("name: acme-live\n"));
        let text = summary_text(&p);
        assert!(text.contains("copy .env.example → .env"));
        assert!(text.contains("commented "));
        assert!(text.contains("create .env.local"));
        assert!(text.contains("set COMPOSE_PROJECT_NAME=acme-live in .env.local"));
        assert!(!text.contains("--vps"));
        assert!(!text.contains("example-secret"));
    }

    #[test]
    fn leftover_shared_deploy_env_migrates_to_local_and_is_commented() {
        let shop = TempShop::new("migrate-env");
        shop.write_env(
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=staging
",
        );
        let mut a = args();
        a.shop_id = None;
        let p = plan(&process(shop.path()), shop.path(), &a).unwrap();
        assert_eq!(p.deploy_env, "staging");
        assert!(p.local_created);
        assert!(p.deploy_env_changed);
        assert!(p.deploy_env_commented >= 1);
        assert_no_uncommented(p.contents(), "SHOPWARE_DEPLOY_ENV");
        assert!(p
            .contents()
            .contains("# SHOPWARE_DEPLOY_ENV=staging # commented by fyrst-cli"));
        assert_eq!(
            last_value(p.local_contents(), "SHOPWARE_DEPLOY_ENV"),
            "staging"
        );
        assert_eq!(
            last_value(p.local_contents(), "COMPOSE_PROJECT_NAME"),
            "acme-staging"
        );
        let text = summary_text(&p);
        assert!(text.contains("set SHOPWARE_DEPLOY_ENV=staging in .env.local"));
        assert!(text.contains("set COMPOSE_PROJECT_NAME=acme-staging in .env.local"));
        assert!(text.contains("commented "));
        assert!(text.contains("SHOPWARE_DEPLOY_ENV"));
    }

    #[test]
    fn leaves_existing_app_secret_unchanged() {
        let shop = TempShop::new("keep-secret");
        shop.write_env(&format!(
            "\
SHOPWARE_SHOP_ID=acme
APP_SECRET={EXISTING_SECRET}
MYSQL_PASSWORD={MYSQL_PASS}
"
        ));
        let mut a = args();
        a.shop_id = None;
        a.dry_run = false;
        let p = plan(&process(shop.path()), shop.path(), &a).unwrap();
        assert_eq!(last_value(p.contents(), "APP_SECRET"), EXISTING_SECRET);
        let text = summary_text(&p);
        assert!(!text.contains(EXISTING_SECRET));
        assert!(!text.contains(MYSQL_PASS));
        assert!(!text.contains("generate-app-secret"));
        assert!(!p.shop_id_changed);
        assert_eq!(
            last_value(p.local_contents(), "SHOPWARE_DEPLOY_ENV"),
            "live"
        );
    }

    #[test]
    fn does_not_generate_empty_app_secret() {
        let shop = TempShop::new("empty-secret");
        shop.write_env(
            "\
SHOPWARE_SHOP_ID=acme
APP_SECRET=
MYSQL_PASSWORD=super-secret-pass
",
        );
        let mut a = args();
        a.dry_run = false;
        let p = plan(&process(shop.path()), shop.path(), &a).unwrap();
        assert_eq!(last_value(p.contents(), "APP_SECRET"), "");
        assert!(p.contents().contains("APP_SECRET="));
        let text = summary_text(&p);
        assert!(!text.contains("set APP_SECRET"));
        assert!(!text.contains("generate-app-secret"));
        assert!(!text.contains("super-secret-pass"));
    }

    #[test]
    fn missing_shop_id_is_fail_not_stub() {
        let shop = TempShop::new("noid");
        shop.write_env("MYSQL_PASSWORD=x\n");
        let mut a = args();
        a.shop_id = None;
        let err = plan(&process(shop.path()), shop.path(), &a).unwrap_err();
        match err {
            Error::Fail(m) => {
                assert!(m.contains("SHOPWARE_SHOP_ID is empty"), "{m}");
                assert!(m.contains("--shop-id"), "{m}");
            }
            Error::NotImplemented(m) => panic!("stub: {m}"),
        }
    }

    #[test]
    fn invalid_slug_and_env_and_image() {
        let shop = TempShop::new("bad");
        shop.write_env("SHOPWARE_SHOP_ID=\nSHOPWARE_DEPLOY_ENV=prod\n");
        let err = plan(&process(shop.path()), shop.path(), &args()).unwrap_err();
        match err {
            Error::Fail(m) => assert!(m.contains("Invalid --env 'prod'"), "{m}"),
            other => panic!("{other:?}"),
        }

        shop.write_env("SHOPWARE_SHOP_ID=\n");
        let mut a = args();
        a.shop_id = Some("ACME".into());
        let err = plan(&process(shop.path()), shop.path(), &a).unwrap_err();
        match err {
            Error::Fail(m) => assert!(m.contains("Invalid --shop-id 'ACME'"), "{m}"),
            other => panic!("{other:?}"),
        }

        a.shop_id = Some("acme".into());
        a.image = Some("ghcr.io/example/acme with space".into());
        let err = plan(&process(shop.path()), shop.path(), &a).unwrap_err();
        match err {
            Error::Fail(m) => assert!(m.contains("no whitespace"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn copy_from_example_when_env_missing() {
        let shop = TempShop::new("copy");
        shop.write_example(
            "\
SHOPWARE_SHOP_ID=
SHOPWARE_DEPLOY_ENV=live
MYSQL_PASSWORD=example-secret
",
        );
        let p = plan(&process(shop.path()), shop.path(), &args()).unwrap();
        assert!(p.copied);
        assert!(p.merged_keys.is_empty());
        assert_eq!(last_value(p.contents(), "MYSQL_PASSWORD"), "example-secret");
        assert_no_uncommented(p.contents(), "COMPOSE_PROJECT_NAME");
        assert_no_uncommented(p.contents(), "SHOPWARE_DEPLOY_ENV");
        assert!(p.local_created);
        assert_eq!(
            last_value(p.local_contents(), "SHOPWARE_DEPLOY_ENV"),
            "live"
        );
        assert_eq!(
            last_value(p.local_contents(), "COMPOSE_PROJECT_NAME"),
            "acme-live"
        );
        let text = summary_text(&p);
        assert!(text.contains("copy .env.example → .env"));
        assert!(text.contains("create .env.local"));
        assert!(text.contains("set COMPOSE_PROJECT_NAME=acme-live in .env.local"));
        assert!(!text.contains("example-secret"));
        assert!(!shop.path().join(".env").is_file());
    }

    #[test]
    fn keep_existing_host_deploy_env() {
        let shop = TempShop::new("keep-env");
        shop.write_env(
            "\
SHOPWARE_SHOP_ID=acme
",
        );
        shop.write_local("SHOPWARE_DEPLOY_ENV=playground\nCOMPOSE_PROJECT_NAME=acme-playground\n");
        shop.write_override("name: acme-playground\n");
        let mut a = args();
        a.shop_id = None;
        let p = plan(&process(shop.path()), shop.path(), &a).unwrap();
        assert_eq!(p.deploy_env, "playground");
        assert!(!p.deploy_env_changed);
        assert!(!p.compose_project_name_changed);
        assert!(!p.override_changed);
        assert!(!p.local_created);
        let text = summary_text(&p);
        assert!(text.contains("already set SHOPWARE_DEPLOY_ENV=playground in .env.local"));
        assert!(text.contains("already set COMPOSE_PROJECT_NAME=acme-playground in .env.local"));
        assert!(!text.contains("--vps"));
        assert!(text.contains("no changes (already up to date)"));
    }

    #[test]
    fn env_flag_overwrites_host_file() {
        let shop = TempShop::new("flag-wins");
        shop.write_env("SHOPWARE_SHOP_ID=acme\n");
        shop.write_local("SHOPWARE_DEPLOY_ENV=live\nCOMPOSE_PROJECT_NAME=acme-live\n");
        shop.write_override("name: acme-live\nservices:\n  web:\n    image: x\n");
        let mut a = args();
        a.shop_id = None;
        a.env = Some(DeployEnv::Dev);
        let p = plan(&process(shop.path()), shop.path(), &a).unwrap();
        assert_eq!(p.deploy_env, "dev");
        assert!(p.deploy_env_changed);
        assert!(p.compose_project_name_changed);
        assert_eq!(last_value(p.local_contents(), "SHOPWARE_DEPLOY_ENV"), "dev");
        assert_eq!(
            last_value(p.local_contents(), "COMPOSE_PROJECT_NAME"),
            "acme-dev"
        );
        assert!(p.override_contents().contains("name: acme-dev\n"));
        assert!(p.override_contents().contains("    image: x"));
        assert!(!p.override_contents().contains("name: acme-live"));
        let text = summary_text(&p);
        assert!(text.contains("set SHOPWARE_DEPLOY_ENV=dev in .env.local"));
        assert!(text.contains("set COMPOSE_PROJECT_NAME=acme-dev in .env.local"));
        assert!(text.contains("set name: acme-dev in compose.override.yaml"));
    }

    #[test]
    fn execute_writes_local_project_name_and_keeps_override_services() {
        let shop = TempShop::new("exec-override");
        shop.write_env(
            "\
SHOPWARE_SHOP_ID=acme
COMPOSE_PROJECT_NAME=sw-shop-acme
",
        );
        shop.write_override(
            "\
# keep me
services:
  web:
    name: nested-must-stay
    ports:
      - \"9003:9003\"
",
        );
        let mut a = args();
        a.dry_run = false;
        a.env = Some(DeployEnv::Dev);
        let p = plan(&process(shop.path()), shop.path(), &a).unwrap();
        execute(&p).unwrap();

        let env = fs::read_to_string(shop.path().join(".env")).unwrap();
        assert_no_uncommented(&env, "COMPOSE_PROJECT_NAME");
        assert!(!env
            .lines()
            .any(|l| l.trim_start().starts_with("COMPOSE_PROJECT_NAME=")));
        let local = fs::read_to_string(shop.path().join(".env.local")).unwrap();
        assert_eq!(last_value(&local, "SHOPWARE_DEPLOY_ENV"), "dev");
        assert_eq!(last_value(&local, "COMPOSE_PROJECT_NAME"), "acme-dev");
        let ov = fs::read_to_string(shop.path().join(OVERRIDE_REL)).unwrap();
        assert!(ov.contains("name: acme-dev\n"));
        assert!(ov.contains("    name: nested-must-stay"));
        assert!(ov.contains("# keep me"));
        assert!(ov.contains("      - \"9003:9003\""));
        #[cfg(unix)]
        {
            let local_mode = fs::metadata(shop.path().join(".env.local"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(local_mode, 0o600);
        }
        assert!(!fs::read_to_string(shop.path().join(".env"))
            .unwrap()
            .lines()
            .any(|l| l.trim_start().starts_with("SHOPWARE_DEPLOY_ENV=")));
    }

    #[test]
    fn slug_pattern() {
        assert!(is_shop_slug("a"));
        assert!(is_shop_slug("acme"));
        assert!(is_shop_slug("acme-shop"));
        assert!(is_shop_slug("a1"));
        assert!(!is_shop_slug(""));
        assert!(!is_shop_slug("ACME"));
        assert!(!is_shop_slug("-acme"));
        assert!(!is_shop_slug("acme-"));
        assert!(!is_shop_slug("acme_shop"));
    }
}
