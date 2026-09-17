//! Shared VPS rollout helpers (recipes `deploy/lib/vps-common.sh`).
//!
//! Used by `shopware deploy release` and `shopware deploy rollback`.

use super::compose::{
    args_contain_build, compose_service_names, docker_log, extend_profiles,
    require_vps_compose_files, run_compose, vps_compose_argv,
};
use super::env::{require_shop_id, vps_project_name, ShopEnv, DEFAULT_DATA_BASE};
use super::error::Error;
use super::mysql::log_contains_secret;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

pub const SMOKE_ATTEMPTS: u32 = 30;
pub const SMOKE_SLEEP: Duration = Duration::from_secs(2);

/// Operator one-liner printed on smoke failure (fyrst-cli equivalent of overlay
/// `IMAGE_TAG=$(cat .previous-tag) bash deploy/vps-rollback.sh`).
pub const ROLLBACK_ONE_LINER: &str =
    "IMAGE_TAG=$(cat .previous-tag) fyrst-cli shopware deploy rollback";

pub fn env_truthy(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub fn env_falsy(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

pub fn apply_pull_policy(skip_pull: bool, pull_policy: Option<&str>) -> (String, bool) {
    if skip_pull {
        return ("never".into(), true);
    }
    let p = pull_policy.unwrap_or("always").trim().to_ascii_lowercase();
    let p = if p.is_empty() {
        "always".to_string()
    } else {
        p
    };
    if p == "never" {
        (p, true)
    } else {
        (p, false)
    }
}

pub fn parse_profiles(raw: Option<&str>) -> Result<Vec<String>, Error> {
    let mut out = Vec::new();
    let Some(raw) = raw.filter(|s| !s.is_empty()) else {
        return Ok(out);
    };
    for part in raw.split(',') {
        let p: String = part.chars().filter(|c| !c.is_whitespace()).collect();
        if p.is_empty() {
            continue;
        }
        if p == "setup" {
            return Err(Error::fail(
                "COMPOSE_PROFILES must not include setup (the script runs that profile itself)",
            ));
        }
        out.push(p);
    }
    Ok(out)
}

/// `ROLLBACK_ON_SMOKE_FAIL`: explicit 1/0 wins. Unset → on for live, off otherwise.
pub fn should_auto_rollback(flag: Option<&str>, deploy_env: &str) -> bool {
    if let Some(f) = flag.filter(|s| !s.is_empty()) {
        if env_truthy(f) {
            return true;
        }
        if env_falsy(f) {
            return false;
        }
    }
    deploy_env.eq_ignore_ascii_case("live")
}

pub fn live_empty_profiles_warnings(deploy_env: &str, profiles_raw: Option<&str>) -> Vec<String> {
    if !deploy_env.eq_ignore_ascii_case("live") {
        return Vec::new();
    }
    if profiles_raw.map(|s| !s.is_empty()).unwrap_or(false) {
        return Vec::new();
    }
    vec![
        "SHOPWARE_DEPLOY_ENV=live but COMPOSE_PROFILES is empty.".into(),
        "redis / worker / scheduler will not start. Recommended live default:".into(),
        "  COMPOSE_PROFILES=redis,worker,scheduler".into(),
        "Uncomment that in .env (do not auto-enable). See deploy/README.md.".into(),
    ]
}

pub fn read_tag_file(path: &Path) -> Option<String> {
    let s = fs::read_to_string(path).ok()?;
    let t: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

pub fn write_tag_file(path: &Path, tag: &str) -> Result<(), Error> {
    fs::write(path, format!("{tag}\n"))
        .map_err(|e| Error::fail(format!("cannot write {}: {e}", path.display())))
}

pub fn record_previous_tag(compose_dir: &Path) -> Result<Option<String>, Error> {
    let src = compose_dir.join(".deployed-tag");
    if !src.is_file() {
        return Ok(None);
    }
    let dst = compose_dir.join(".previous-tag");
    fs::copy(&src, &dst)
        .map_err(|e| Error::fail(format!("cannot copy .deployed-tag to .previous-tag: {e}")))?;
    Ok(read_tag_file(&dst))
}

pub fn up_pull_args(skip_pull: bool) -> Vec<String> {
    if skip_pull {
        vec!["--pull".into(), "never".into()]
    } else {
        Vec::new()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RolloutPlan {
    pub skip_pull: bool,
    pub pull_policy: String,
    pub profiles: Vec<String>,
    pub profiles_raw: String,
    pub pull: Option<Vec<String>>,
    pub setup: Vec<String>,
    pub web: Vec<String>,
    pub extra: Option<Vec<String>>,
}

impl RolloutPlan {
    pub fn build(
        compose_dir: &Path,
        skip_pull: bool,
        pull_policy: String,
        profiles: Vec<String>,
        profiles_raw: String,
    ) -> Self {
        let pull = if skip_pull {
            None
        } else {
            let mut a = vps_compose_argv(compose_dir);
            extend_profiles(&mut a, &profiles);
            a.push("pull".into());
            Some(a)
        };
        let mut setup = vps_compose_argv(compose_dir);
        setup.extend([
            "--profile".into(),
            "setup".into(),
            "run".into(),
            "--rm".into(),
            "--pull".into(),
            "never".into(),
            "setup".into(),
        ]);
        let mut web = vps_compose_argv(compose_dir);
        web.extend(["up".into(), "-d".into(), "--no-build".into()]);
        web.extend(up_pull_args(skip_pull));
        web.extend(["--remove-orphans".into(), "web".into()]);
        let extra = if profiles.is_empty() {
            None
        } else {
            let mut a = vps_compose_argv(compose_dir);
            extend_profiles(&mut a, &profiles);
            a.extend(["up".into(), "-d".into(), "--no-build".into()]);
            a.extend(up_pull_args(skip_pull));
            Some(a)
        };
        Self {
            skip_pull,
            pull_policy,
            profiles,
            profiles_raw,
            pull,
            setup,
            web,
            extra,
        }
    }

    pub fn mutating_args(&self) -> Vec<&[String]> {
        let mut v = Vec::new();
        if let Some(p) = &self.pull {
            v.push(p.as_slice());
        }
        v.push(self.setup.as_slice());
        v.push(self.web.as_slice());
        if let Some(e) = &self.extra {
            v.push(e.as_slice());
        }
        v
    }

    pub fn never_builds(&self) -> bool {
        self.mutating_args()
            .into_iter()
            .all(|a| !args_contain_build(a))
    }
}

pub struct VpsContext {
    pub compose_dir: PathBuf,
    pub image: String,
    pub image_tag: String,
    pub shop_id: String,
    pub deploy_env: String,
    pub compose_project_name: String,
    pub data_base: String,
    pub data_root: String,
    pub profiles: Vec<String>,
    pub profiles_raw: String,
    pub smoke_url: Option<String>,
    pub pull_policy: String,
    pub skip_pull: bool,
    pub rollback_on_smoke_fail: Option<String>,
    pub dry_run: bool,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
    secrets: Vec<String>,
}

impl VpsContext {
    pub fn extra_env(&self) -> Vec<(String, String)> {
        let mut v = vec![
            ("IMAGE".into(), self.image.clone()),
            ("IMAGE_TAG".into(), self.image_tag.clone()),
            ("PULL_POLICY".into(), self.pull_policy.clone()),
            (
                "SKIP_PULL".into(),
                if self.skip_pull {
                    "1".into()
                } else {
                    "0".into()
                },
            ),
            ("SHOPWARE_SHOP_ID".into(), self.shop_id.clone()),
            ("SHOPWARE_DEPLOY_ENV".into(), self.deploy_env.clone()),
            ("SHOPWARE_DATA_BASE".into(), self.data_base.clone()),
            ("SHOPWARE_DATA_ROOT".into(), self.data_root.clone()),
        ];
        if !self.profiles_raw.is_empty() {
            v.push(("COMPOSE_PROFILES".into(), self.profiles_raw.clone()));
        }
        v
    }

    pub fn plan(&self) -> RolloutPlan {
        RolloutPlan::build(
            &self.compose_dir,
            self.skip_pull,
            self.pull_policy.clone(),
            self.profiles.clone(),
            self.profiles_raw.clone(),
        )
    }

    pub fn should_auto_rollback(&self) -> bool {
        should_auto_rollback(self.rollback_on_smoke_fail.as_deref(), &self.deploy_env)
    }

    pub fn log_contains_secret(&self, text: &str) -> bool {
        log_contains_secret(text, &self.secrets)
    }
}

pub fn bootstrap(env: &ShopEnv, skip_pull_flag: bool, dry_run: bool) -> Result<VpsContext, Error> {
    require_vps_compose_files(&env.compose_dir)?;
    let image = env
        .get("IMAGE")
        .map(str::to_string)
        .ok_or_else(|| Error::fail("Set IMAGE to the registry repository"))?;
    let image_tag = env.get("IMAGE_TAG").map(str::to_string).ok_or_else(|| {
        Error::fail("Set IMAGE_TAG to the git SHA (or previous tag for rollback)")
    })?;
    let shop_id = require_shop_id(env)?;
    let deploy_env = env
        .get("SHOPWARE_DEPLOY_ENV")
        .map(str::to_string)
        .ok_or_else(|| {
            Error::fail("Set SHOPWARE_DEPLOY_ENV in .env.local (live|staging|playground|dev)")
        })?;

    let compose_project_name = vps_project_name(&shop_id, &deploy_env);
    let data_base = env
        .get("SHOPWARE_DATA_BASE")
        .unwrap_or(DEFAULT_DATA_BASE)
        .to_string();
    let (data_root, data_root_derived) = if let Some(r) = env.get("SHOPWARE_DATA_ROOT") {
        (r.to_string(), false)
    } else {
        (format!("{data_base}/{shop_id}/{deploy_env}"), true)
    };

    let skip_from_env = env.get("SKIP_PULL").map(env_truthy).unwrap_or(false);
    let (pull_policy, skip_pull) =
        apply_pull_policy(skip_pull_flag || skip_from_env, env.get("PULL_POLICY"));

    let profiles_raw = env.get("COMPOSE_PROFILES").unwrap_or("").to_string();
    let profiles = parse_profiles(if profiles_raw.is_empty() {
        None
    } else {
        Some(profiles_raw.as_str())
    })?;

    let mut notes = Vec::new();
    let warnings = live_empty_profiles_warnings(&deploy_env, env.get("COMPOSE_PROFILES"));
    if let Some(from_env) = env.get("COMPOSE_PROJECT_NAME") {
        if from_env != compose_project_name {
            notes.push(format!(
                "COMPOSE_PROJECT_NAME={from_env} in env files is ignored; VPS project is deploy/compose.yaml name: interpolating host env files ({compose_project_name})"
            ));
        }
    } else {
        notes.push(format!(
            "compose project {compose_project_name} from deploy/compose.yaml name: + host env files"
        ));
    }
    if data_root_derived {
        notes.push(format!("SHOPWARE_DATA_ROOT unset; derived {data_root}"));
    }

    let mut secrets = Vec::new();
    for key in [
        "MYSQL_PASSWORD",
        "MYSQL_ROOT_PASSWORD",
        "APP_SECRET",
        "DATABASE_URL",
    ] {
        if let Some(v) = env.get(key) {
            secrets.push(v.to_string());
        }
    }

    Ok(VpsContext {
        compose_dir: env.compose_dir.clone(),
        image,
        image_tag,
        shop_id,
        deploy_env,
        compose_project_name,
        data_base,
        data_root,
        profiles,
        profiles_raw,
        smoke_url: env.get("SMOKE_URL").map(str::to_string),
        pull_policy,
        skip_pull,
        rollback_on_smoke_fail: env.get("ROLLBACK_ON_SMOKE_FAIL").map(str::to_string),
        dry_run,
        notes,
        warnings,
        secrets,
    })
}

pub fn ensure_env_prod(compose_dir: &Path) -> Result<(), Error> {
    let p = compose_dir.join(".env.prod");
    if !p.exists() {
        fs::write(&p, "").map_err(|e| Error::fail(format!("cannot create .env.prod: {e}")))?;
    }
    Ok(())
}

pub fn execute_rollout(ctx: &VpsContext, plan: &RolloutPlan) -> Result<(), Error> {
    println!(
        "==> Deploying {}:{} from {} (never compile themes/assets; compose run --pull never, up --no-build)",
        ctx.image,
        ctx.image_tag,
        ctx.compose_dir.display()
    );
    let env = ctx.extra_env();
    if plan.skip_pull {
        println!(
            "==> Skipping registry pull (SKIP_PULL=1 / PULL_POLICY={}); using images already on this host",
            plan.pull_policy
        );
    } else if let Some(pull) = &plan.pull {
        println!("==> Pulling images");
        run_compose(&ctx.compose_dir, pull, &env)?;
    }

    let mut profile_args = Vec::new();
    extend_profiles(&mut profile_args, &plan.profiles);
    let services = compose_service_names(&ctx.compose_dir, &profile_args, &env);

    if services.iter().any(|s| s == "mysql") {
        println!("==> Starting mysql");
        let mut a = vps_compose_argv(&ctx.compose_dir);
        a.extend(["up".into(), "-d".into(), "--no-build".into()]);
        a.extend(up_pull_args(plan.skip_pull));
        a.push("mysql".into());
        run_compose(&ctx.compose_dir, &a, &env)?;
    }

    if services.iter().any(|s| s == "redis") || ctx.profiles_raw.contains("redis") {
        println!("==> Starting redis");
        let mut a = vps_compose_argv(&ctx.compose_dir);
        a.extend([
            "--profile".into(),
            "redis".into(),
            "up".into(),
            "-d".into(),
            "--no-build".into(),
        ]);
        a.extend(up_pull_args(plan.skip_pull));
        a.push("redis".into());
        run_compose(&ctx.compose_dir, &a, &env)?;
    }

    println!("==> One-shot setup (shopware-deployment-helper, skip theme/assets)");
    run_compose(&ctx.compose_dir, &plan.setup, &env)?;

    println!("==> Recreating web (no build)");
    run_compose(&ctx.compose_dir, &plan.web, &env)?;

    if let Some(extra) = &plan.extra {
        println!("==> Starting extra profiles: {}", plan.profiles_raw);
        run_compose(&ctx.compose_dir, extra, &env)?;
    }
    Ok(())
}

pub fn smoke(ctx: &VpsContext) -> Result<(), Error> {
    let Some(url) = ctx.smoke_url.as_deref() else {
        return Ok(());
    };
    println!("==> Smoke {url}");
    if ctx.dry_run {
        println!("==> DRY-RUN would GET {url}");
        return Ok(());
    }
    let curl_ok = Command::new("curl")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !curl_ok {
        return Err(Error::fail(format!(
            "Smoke check failed for {url}: curl is not available"
        )));
    }
    for _ in 0..SMOKE_ATTEMPTS {
        let ok = Command::new("curl")
            .args(["-fsS", url])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            println!("==> Smoke OK");
            return Ok(());
        }
        thread::sleep(SMOKE_SLEEP);
    }
    Err(Error::fail(format!("Smoke check failed for {url}")))
}

pub fn smoke_fail_message(url: &str) -> String {
    format!("Smoke check failed for {url}\nERROR: Rollback command: {ROLLBACK_ONE_LINER}")
}

/// Re-run the same compose order at `image_tag` (release auto-rollback).
/// Shared helper used by `fyrst-cli shopware deploy rollback`.
pub fn rollout_to_tag(ctx: &mut VpsContext, image_tag: String) -> Result<(), Error> {
    ctx.image_tag = image_tag;
    let plan = ctx.plan();
    execute_rollout(ctx, &plan)?;
    smoke(ctx)?;
    write_tag_file(&ctx.compose_dir.join(".deployed-tag"), &ctx.image_tag)?;
    Ok(())
}

pub fn docker_log_checked(ctx: &VpsContext, args: &[String]) -> Result<String, Error> {
    let line = docker_log(args);
    if ctx.log_contains_secret(&line) {
        return Err(Error::fail(
            "internal error: release log would have included a password",
        ));
    }
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn plan(skip: bool, profiles: &str) -> (TempShop, RolloutPlan) {
        let shop = TempShop::new("plan");
        let parsed = parse_profiles(if profiles.is_empty() {
            None
        } else {
            Some(profiles)
        })
        .unwrap();
        let p = RolloutPlan::build(
            &shop.0,
            skip,
            if skip {
                "never".into()
            } else {
                "always".into()
            },
            parsed,
            profiles.to_string(),
        );
        (shop, p)
    }

    #[test]
    fn skip_pull_sets_never_and_omits_pull() {
        let (policy, skip) = apply_pull_policy(true, Some("always"));
        assert_eq!(policy, "never");
        assert!(skip);
        let (_shop, p) = plan(true, "");
        assert!(p.pull.is_none());
        assert!(p.web.windows(2).any(|w| w == ["--pull", "never"]));
        assert!(p.web.iter().any(|a| a == "--no-build"));
        assert!(!p.web.iter().any(|a| a == "-p" || a == "--project-name"));
        assert!(!p.setup.iter().any(|a| a == "--no-build"));
        assert!(p.setup.windows(2).any(|w| w == ["--pull", "never"]));
        assert!(p.never_builds());
    }

    #[test]
    fn pull_policy_never_skips_without_flag() {
        let (policy, skip) = apply_pull_policy(false, Some("never"));
        assert_eq!(policy, "never");
        assert!(skip);
        let (_shop, p) = plan(false, "redis,worker");
        assert!(p.never_builds());
        let pull = p.pull.clone().expect("pull");
        assert!(pull.ends_with(&["pull".to_string()]));
        assert!(pull.windows(2).any(|w| w == ["--profile", "redis"]));
        assert!(!p.web.windows(2).any(|w| w == ["--pull", "never"]));
        assert!(p.extra.is_some());
        assert!(p.never_builds());
    }

    #[test]
    fn setup_profile_refused() {
        let err = parse_profiles(Some("redis,setup")).unwrap_err();
        assert!(err.to_string().contains("setup"), "{err}");
    }

    #[test]
    fn profiles_strip_spaces() {
        assert_eq!(
            parse_profiles(Some("redis, worker, scheduler")).unwrap(),
            vec!["redis", "worker", "scheduler"]
        );
    }

    #[test]
    fn auto_rollback_live_default() {
        assert!(should_auto_rollback(None, "live"));
        assert!(should_auto_rollback(None, "LIVE"));
        assert!(!should_auto_rollback(None, "staging"));
        assert!(!should_auto_rollback(Some("0"), "live"));
        assert!(should_auto_rollback(Some("1"), "staging"));
        assert!(should_auto_rollback(Some("true"), "dev"));
        assert!(!should_auto_rollback(Some("off"), "live"));
    }

    #[test]
    fn live_empty_profiles_warns() {
        let w = live_empty_profiles_warnings("live", None);
        assert!(w
            .iter()
            .any(|s| s.contains("COMPOSE_PROFILES=redis,worker,scheduler")));
        assert!(live_empty_profiles_warnings("live", Some("redis")).is_empty());
        assert!(live_empty_profiles_warnings("staging", None).is_empty());
    }

    #[test]
    fn smoke_fail_prints_rollback_one_liner() {
        let m = smoke_fail_message("http://127.0.0.1:8000");
        assert!(m.contains("http://127.0.0.1:8000"), "{m}");
        assert!(m.contains(ROLLBACK_ONE_LINER), "{m}");
        assert!(m.contains("fyrst-cli shopware deploy rollback"), "{m}");
        assert!(!m.contains("DATABASE_URL"), "{m}");
    }

    #[test]
    fn setup_run_never_uses_no_build() {
        let (_shop, p) = plan(false, "");
        let log = docker_log(&p.setup);
        assert!(
            log.contains("--profile setup run --rm --pull never setup"),
            "{log}"
        );
        assert!(!log.contains("--no-build"), "{log}");
        let web = docker_log(&p.web);
        assert!(
            web.contains("up -d --no-build --remove-orphans web"),
            "{web}"
        );
        assert!(
            !web.split_whitespace()
                .any(|t| t == "-p" || t == "--project-name"),
            "{web}"
        );
        assert!(!web.contains("shopware-acme"), "{web}");
    }

    struct TempShop(PathBuf);
    impl TempShop {
        fn new(prefix: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let p = std::env::temp_dir().join(format!(
                "fyrst-cli-roll-{prefix}-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(p.join("deploy")).unwrap();
            Self(p)
        }
    }
    impl Drop for TempShop {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_vps_files(shop: &Path) {
        for f in [
            "deploy/compose.yaml",
            "deploy/compose.prod.yaml",
            "deploy/compose.vps.yaml",
        ] {
            fs::write(shop.join(f), "services:\n  web:\n    image: x\n").unwrap();
        }
    }

    #[test]
    fn bootstrap_skip_pull_and_ci_tag() {
        let shop = TempShop::new("boot");
        write_vps_files(&shop.0);
        fs::write(
            shop.0.join(".env"),
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=staging
IMAGE=ghcr.io/file/shop
IMAGE_TAG=latest
COMPOSE_PROJECT_NAME=shopware-acme
MYSQL_PASSWORD=super-secret-pass
DATABASE_URL=mysql://alice:s3cret-value@db.example.com/shop
",
        )
        .unwrap();
        let mut process = HashMap::new();
        process.insert("IMAGE_TAG".into(), "abc123".into());
        let env = ShopEnv::load(shop.0.clone(), &process).unwrap();
        let ctx = bootstrap(&env, true, true).unwrap();
        assert_eq!(ctx.image_tag, "abc123");
        assert_eq!(ctx.image, "ghcr.io/file/shop");
        assert!(ctx.skip_pull);
        assert_eq!(ctx.pull_policy, "never");
        assert_eq!(ctx.compose_project_name, "acme-staging");
        assert!(ctx.notes.iter().any(|n| n.contains("acme-staging")));
        assert!(ctx
            .notes
            .iter()
            .any(|n| n.contains("COMPOSE_PROJECT_NAME=shopware-acme")));
        assert!(ctx
            .notes
            .iter()
            .any(|n| n.contains("deploy/compose.yaml name:") && n.contains("acme-staging")));
        assert!(!ctx.notes.iter().any(|n| n.contains("-p ")));
        assert!(!ctx
            .extra_env()
            .iter()
            .any(|(k, _)| k == "COMPOSE_PROJECT_NAME"));
        assert!(!ctx.warnings.iter().any(|w| w.contains("remove or comment")));
        let plan = ctx.plan();
        assert!(plan.pull.is_none());
        assert!(!plan.web.iter().any(|a| a == "-p" || a == "--project-name"));
        let web = docker_log_checked(&ctx, &plan.web).unwrap();
        assert!(!ctx.log_contains_secret(&web));
        assert!(!web.contains("s3cret-value"));
        assert!(!web.contains("super-secret-pass"));
    }

    #[test]
    fn rollout_plan_passes_host_env_files() {
        let shop = TempShop::new("host-env");
        write_vps_files(&shop.0);
        fs::write(shop.0.join(".env.local"), "SHOPWARE_DEPLOY_ENV=dev\n").unwrap();
        fs::write(shop.0.join(".env.prod"), "").unwrap();
        let p = RolloutPlan::build(&shop.0, true, "never".into(), Vec::new(), String::new());
        assert!(p.web.windows(2).any(|w| w == ["--env-file", ".env"]));
        assert!(p.web.windows(2).any(|w| w == ["--env-file", ".env.local"]));
        assert!(p.web.windows(2).any(|w| w == ["--env-file", ".env.prod"]));
        assert!(!p.web.iter().any(|a| a == "-p" || a == "--project-name"));
        let log = docker_log(&p.web);
        assert!(
            log.contains("--env-file .env --env-file .env.local --env-file .env.prod"),
            "{log}"
        );
        assert!(log.contains("-f deploy/compose.yaml"), "{log}");
        assert!(!log.split_whitespace().any(|t| t == "-p"), "{log}");
    }

    #[test]
    fn missing_vps_file_errors() {
        let shop = TempShop::new("nofile");
        fs::write(
            shop.0.join("deploy/compose.yaml"),
            "services:\n  web:\n    image: x\n",
        )
        .unwrap();
        fs::write(
            shop.0.join("deploy/compose.prod.yaml"),
            "services:\n  web:\n    image: x\n",
        )
        .unwrap();
        fs::write(
            shop.0.join(".env"),
            "SHOPWARE_SHOP_ID=acme\nSHOPWARE_DEPLOY_ENV=dev\nIMAGE=x\nIMAGE_TAG=t\n",
        )
        .unwrap();
        let env = ShopEnv::load(shop.0.clone(), &HashMap::new()).unwrap();
        let err = match bootstrap(&env, false, true) {
            Err(e) => e,
            Ok(_) => panic!("expected missing compose.vps.yaml"),
        };
        assert!(err.to_string().contains("compose.vps.yaml"), "{err}");
    }

    #[test]
    fn record_previous_from_deployed() {
        let shop = TempShop::new("prev");
        fs::write(shop.0.join(".deployed-tag"), "oldtag\n").unwrap();
        let prev = record_previous_tag(&shop.0).unwrap();
        assert_eq!(prev.as_deref(), Some("oldtag"));
        assert_eq!(
            read_tag_file(&shop.0.join(".previous-tag")).as_deref(),
            Some("oldtag")
        );
    }
}
