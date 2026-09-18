//! `fyrst-cli shopware deploy rollback` — recipes `deploy/vps-rollback.sh`.
//!
//! `IMAGE` stays from env / `.env`. `IMAGE_TAG` is **only** `.previous-tag`
//! (process-env `IMAGE_TAG` is ignored). Reuses the release compose/rollout
//! helper. Writes `.deployed-tag` only after a successful rollout (and deploy
//! health probe, if set).

use super::env::{resolve_compose_dir, ShopEnv};
use super::error::Error;
use super::mysql::require_docker;
use super::rollout::{
    bootstrap, deploy_health, docker_log_checked, ensure_env_prod, execute_rollout, read_tag_file,
    write_tag_file, VpsContext,
};
use crate::cli::RollbackArgs;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub fn run(args: RollbackArgs) -> Result<(), Error> {
    let mut process: HashMap<String, String> = std::env::vars().collect();
    if args.skip_pull {
        process.insert("SKIP_PULL".into(), "1".into());
    }
    if args.allow_no_deploy_health {
        process.insert("ALLOW_NO_DEPLOY_HEALTH".into(), "1".into());
    }
    let cwd = std::env::current_dir().map_err(|e| Error::fail(format!("cannot read cwd: {e}")))?;
    execute_rollback(&process, &cwd, args.skip_pull, args.dry_run)
}

pub fn execute_rollback(
    process_env: &HashMap<String, String>,
    cwd: &Path,
    skip_pull: bool,
    dry_run: bool,
) -> Result<(), Error> {
    let compose_dir = resolve_compose_dir(process_env, cwd)?;
    let compose_dir = fs::canonicalize(&compose_dir).unwrap_or(compose_dir);

    let image_tag = read_previous_tag(&compose_dir)?;

    let mut process = process_env.clone();
    process.remove("IMAGE_TAG");
    process.insert("IMAGE_TAG".into(), image_tag.clone());

    let env = ShopEnv::load(compose_dir, &process)?;
    let ctx = bootstrap(&env, skip_pull, dry_run)?;

    for w in &ctx.warnings {
        eprintln!("WARNING: {w}");
    }
    for n in &ctx.notes {
        emit(&ctx, format!("==> {n}"))?;
    }
    emit(
        &ctx,
        format!(
            "==> Rollback IMAGE_TAG from .previous-tag: {} (IMAGE={} unchanged)",
            ctx.image_tag, ctx.image
        ),
    )?;
    emit(
        &ctx,
        format!(
            "==> Rollback to {}:{} (same compose stack as vps-release.sh)",
            ctx.image, ctx.image_tag
        ),
    )?;

    ensure_env_prod(&ctx.compose_dir)?;

    let plan = ctx.plan();
    if !plan.never_builds() {
        return Err(Error::fail(
            "internal error: rollout plan included --build (VPS paths never build images)",
        ));
    }

    if dry_run {
        print_dry_run(&ctx, &plan)?;
        if let Some(url) = &ctx.deploy_health_url {
            emit(&ctx, format!("==> DRY-RUN would GET {url}"))?;
        }
        emit(
            &ctx,
            format!("==> DRY-RUN would write .deployed-tag={}", ctx.image_tag),
        )?;
        emit(
            &ctx,
            format!("==> Rollback finished {}:{}", ctx.image, ctx.image_tag),
        )?;
        return Ok(());
    }

    require_docker()?;
    execute_rollout(&ctx, &plan)?;

    if let Err(_e) = deploy_health(&ctx) {
        let url = ctx
            .deploy_health_url
            .as_deref()
            .unwrap_or("DEPLOY_HEALTH_URL");
        return Err(Error::fail(format!(
            "Deploy health check failed for {url} after rollback. Stack is on {}:{} but .deployed-tag was not updated",
            ctx.image, ctx.image_tag
        )));
    }

    write_tag_file(&ctx.compose_dir.join(".deployed-tag"), &ctx.image_tag)?;
    emit(
        &ctx,
        format!("==> Rollback finished {}:{}", ctx.image, ctx.image_tag),
    )?;
    Ok(())
}

fn read_previous_tag(compose_dir: &Path) -> Result<String, Error> {
    let path = compose_dir.join(".previous-tag");
    if !path.is_file() {
        return Err(Error::fail(
            "No .previous-tag (missing). Rollback needs a prior deploy/vps-release.sh run that recorded the running tag. Refusing to guess IMAGE_TAG.",
        ));
    }
    match read_tag_file(&path) {
        Some(t) => Ok(t),
        None => Err(Error::fail(
            ".previous-tag is empty. Rollback refuses to guess IMAGE_TAG.",
        )),
    }
}

fn emit(ctx: &VpsContext, line: String) -> Result<(), Error> {
    if ctx.log_contains_secret(&line) {
        return Err(Error::fail(
            "internal error: rollback log would have included a password",
        ));
    }
    println!("{line}");
    Ok(())
}

fn print_dry_run(ctx: &VpsContext, plan: &super::rollout::RolloutPlan) -> Result<(), Error> {
    emit(
        ctx,
        format!(
            "==> Deploying {}:{} from {} (never compile themes/assets; compose run --pull never, up --no-build)",
            ctx.image,
            ctx.image_tag,
            ctx.compose_dir.display()
        ),
    )?;
    if plan.skip_pull {
        emit(
            ctx,
            format!(
                "==> DRY-RUN skip compose pull (SKIP_PULL=1 / PULL_POLICY={})",
                plan.pull_policy
            ),
        )?;
    } else if let Some(pull) = &plan.pull {
        emit(
            ctx,
            format!("==> DRY-RUN {}", docker_log_checked(ctx, pull)?),
        )?;
    }
    emit(
        ctx,
        "==> DRY-RUN start mysql/redis if present, then:".into(),
    )?;
    emit(
        ctx,
        format!("==> DRY-RUN {}", docker_log_checked(ctx, &plan.setup)?),
    )?;
    emit(
        ctx,
        format!("==> DRY-RUN {}", docker_log_checked(ctx, &plan.web)?),
    )?;
    if plan.extra.is_some() {
        emit(
            ctx,
            format!("==> DRY-RUN extra profiles: {}", plan.profiles_raw),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempShop(PathBuf);
    impl TempShop {
        fn new(prefix: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let p = std::env::temp_dir().join(format!(
                "fyrst-cli-rb-{prefix}-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(p.join("deploy")).unwrap();
            Self(p)
        }
        fn write_vps(&self) {
            for f in [
                "deploy/compose.yaml",
                "deploy/compose.prod.yaml",
                "deploy/compose.vps.yaml",
            ] {
                fs::write(self.0.join(f), "services:\n  web:\n    image: x\n").unwrap();
            }
        }
        fn write_env(&self, extra: &str) {
            fs::write(
                self.0.join(".env"),
                format!(
                    "\
IMAGE=ghcr.io/example/acme
IMAGE_TAG=from-env-file
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=staging
{extra}"
                ),
            )
            .unwrap();
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

    #[test]
    fn previous_tag_required() {
        let shop = TempShop::new("notag");
        shop.write_vps();
        shop.write_env("");
        let err = execute_rollback(&process(&shop.0), &shop.0, false, true).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(".previous-tag"), "{msg}");
        assert!(msg.contains("IMAGE_TAG"), "{msg}");
        assert!(!shop.0.join(".deployed-tag").is_file());
    }

    #[test]
    fn process_image_tag_does_not_override_previous_tag() {
        let shop = TempShop::new("ignore-tag");
        shop.write_vps();
        shop.write_env("");
        fs::write(shop.0.join(".previous-tag"), "rolled-back-sha\n").unwrap();
        let mut process = process(&shop.0);
        process.insert("IMAGE_TAG".into(), "from-process".into());
        process.insert("IMAGE".into(), "ghcr.io/example/acme".into());
        execute_rollback(&process, &shop.0, false, true).unwrap();
        assert!(!shop.0.join(".deployed-tag").is_file());
    }

    #[test]
    fn dry_run_does_not_write_deployed_tag() {
        let shop = TempShop::new("dry-tag");
        shop.write_vps();
        shop.write_env("");
        fs::write(shop.0.join(".previous-tag"), "prev\n").unwrap();
        execute_rollback(&process(&shop.0), &shop.0, false, true).unwrap();
        assert!(!shop.0.join(".deployed-tag").is_file());
    }

    #[test]
    fn skip_pull_from_env() {
        let shop = TempShop::new("skip-env");
        shop.write_vps();
        shop.write_env("SKIP_PULL=1\nCOMPOSE_PROFILES=redis,worker\n");
        fs::write(shop.0.join(".previous-tag"), "t1\n").unwrap();
        execute_rollback(&process(&shop.0), &shop.0, false, true).unwrap();
    }

    #[test]
    fn process_image_wins_over_file() {
        let shop = TempShop::new("img");
        shop.write_vps();
        shop.write_env("");
        fs::write(shop.0.join(".previous-tag"), "t1\n").unwrap();
        let mut process = process(&shop.0);
        process.insert("IMAGE".into(), "ghcr.io/ci/override".into());
        execute_rollback(&process, &shop.0, false, true).unwrap();
    }

    #[test]
    fn missing_compose_vps_yaml() {
        let shop = TempShop::new("nofile");
        fs::write(shop.0.join("deploy/compose.yaml"), "services: {}\n").unwrap();
        fs::write(shop.0.join("deploy/compose.prod.yaml"), "services: {}\n").unwrap();
        shop.write_env("");
        fs::write(shop.0.join(".previous-tag"), "t1\n").unwrap();
        let err = execute_rollback(&process(&shop.0), &shop.0, false, true).unwrap_err();
        assert!(err.to_string().contains("compose.vps.yaml"), "{}", err);
    }
}
