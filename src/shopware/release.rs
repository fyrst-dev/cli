//! `fyrst-cli shopware deploy release` — VPS image pull + Compose recreate.
//!
//! Matches recipes `deploy/vps-release.sh`. Never builds images or compiles
//! themes/assets. Auto-rollback on deploy-health failure uses the shared rollout
//! helper (`fyrst-cli shopware deploy rollback` is the operator command).

use super::env::{resolve_compose_dir, ShopEnv};
use super::error::Error;
use super::rollout::{
    bootstrap, deploy_health, deploy_health_fail_message, docker_log_checked, ensure_env_prod,
    execute_rollout, read_tag_file, record_previous_tag, rollout_to_tag, write_tag_file,
    ROLLBACK_ONE_LINER,
};
use crate::cli::ReleaseArgs;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub fn run(args: ReleaseArgs) -> Result<(), Error> {
    let mut process_env: HashMap<String, String> = std::env::vars().collect();
    if args.skip_pull {
        process_env.insert("SKIP_PULL".into(), "1".into());
    }
    if args.allow_no_deploy_health {
        process_env.insert("ALLOW_NO_DEPLOY_HEALTH".into(), "1".into());
    }
    let cwd = std::env::current_dir().map_err(|e| Error::fail(format!("cannot read cwd: {e}")))?;
    execute_release(&process_env, &cwd, args.skip_pull, args.dry_run)
}

pub fn execute_release(
    process_env: &HashMap<String, String>,
    cwd: &Path,
    skip_pull: bool,
    dry_run: bool,
) -> Result<(), Error> {
    let compose_dir = resolve_compose_dir(process_env, cwd)?;
    let compose_dir = fs::canonicalize(&compose_dir).unwrap_or(compose_dir);
    let env = ShopEnv::load(compose_dir, process_env)?;
    let mut ctx = bootstrap(&env, skip_pull, dry_run)?;

    for w in &ctx.warnings {
        eprintln!("WARNING: {w}");
    }
    for n in &ctx.notes {
        emit(&ctx, format!("==> {n}"))?;
    }

    if !dry_run {
        ensure_env_prod(&ctx.compose_dir)?;
    }

    let previous = if dry_run {
        read_tag_file(&ctx.compose_dir.join(".deployed-tag"))
    } else {
        record_previous_tag(&ctx.compose_dir)?
    };
    if let Some(t) = &previous {
        emit(&ctx, format!("==> Previous tag: {t}"))?;
    }

    let plan = ctx.plan();
    if !plan.never_builds() {
        return Err(Error::fail(
            "internal error: rollout plan included --build (VPS paths never build images)",
        ));
    }

    if dry_run {
        print_dry_run(&ctx, &plan)?;
        emit(
            &ctx,
            format!("==> DRY-RUN would write .deployed-tag={}", ctx.image_tag),
        )?;
        emit(
            &ctx,
            format!("==> Deploy finished {}:{}", ctx.image, ctx.image_tag),
        )?;
        return Ok(());
    }

    execute_rollout(&ctx, &plan)?;

    if let Err(_e) = deploy_health(&ctx) {
        let url = ctx
            .deploy_health_url
            .as_deref()
            .unwrap_or("DEPLOY_HEALTH_URL");
        let mut msg = deploy_health_fail_message(url);
        if ctx.should_auto_rollback() {
            println!(
                "==> ROLLBACK_ON_FAIL on (SHOPWARE_DEPLOY_ENV={}; live default is on, others off unless ROLLBACK_ON_FAIL=1)",
                ctx.deploy_env
            );
            let Some(tag) = previous.filter(|t| !t.is_empty()) else {
                msg.push_str(
                    "\nERROR: Cannot auto-rollback: .previous-tag missing or empty (first deploy, or no recorded prior tag)",
                );
                return Err(Error::fail(msg));
            };
            if let Err(rb) = rollout_to_tag(&mut ctx, tag) {
                msg.push_str(&format!(
                    "\nERROR: Auto-rollback failed. Stack may be on the new tag. Retry: {ROLLBACK_ONE_LINER}"
                ));
                msg.push_str("\nERROR: ");
                msg.push_str(&rb.to_string());
                return Err(Error::fail(msg));
            }
            msg.push_str(
                "\nERROR: Rolled back after deploy health failure. Release still exits 1 so CI does not treat the new tag as live.",
            );
            return Err(Error::fail(msg));
        }
        println!(
            "==> Auto-rollback skipped (SHOPWARE_DEPLOY_ENV={}; set ROLLBACK_ON_FAIL=1 to enable)",
            ctx.deploy_env
        );
        return Err(Error::fail(msg));
    }

    write_tag_file(&ctx.compose_dir.join(".deployed-tag"), &ctx.image_tag)?;
    emit(
        &ctx,
        format!("==> Deploy finished {}:{}", ctx.image, ctx.image_tag),
    )?;
    Ok(())
}

fn emit(ctx: &super::rollout::VpsContext, line: String) -> Result<(), Error> {
    if ctx.log_contains_secret(&line) {
        return Err(Error::fail(
            "internal error: release log would have included a password",
        ));
    }
    println!("{line}");
    Ok(())
}

fn print_dry_run(
    ctx: &super::rollout::VpsContext,
    plan: &super::rollout::RolloutPlan,
) -> Result<(), Error> {
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
    if let Some(url) = &ctx.deploy_health_url {
        emit(ctx, format!("==> DRY-RUN would GET {url}"))?;
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
                "fyrst-cli-rel-{prefix}-{}-{nanos}",
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
    fn dry_run_does_not_write_tag_files() {
        let shop = TempShop::new("dry");
        shop.write_vps();
        fs::write(
            shop.0.join(".env"),
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=staging
IMAGE=ghcr.io/fyrst-dev/acme
IMAGE_TAG=deadbeef
MYSQL_PASSWORD=super-secret-pass
",
        )
        .unwrap();
        fs::write(shop.0.join(".deployed-tag"), "oldtag\n").unwrap();
        execute_release(&process(&shop.0), &shop.0, false, true).unwrap();
        assert!(!shop.0.join(".previous-tag").is_file());
        assert_eq!(
            fs::read_to_string(shop.0.join(".deployed-tag"))
                .unwrap()
                .trim(),
            "oldtag"
        );
        assert!(!shop.0.join(".env.prod").is_file());
    }

    #[test]
    fn setup_in_profiles_fails() {
        let shop = TempShop::new("setup");
        shop.write_vps();
        fs::write(
            shop.0.join(".env"),
            "\
SHOPWARE_SHOP_ID=acme
SHOPWARE_DEPLOY_ENV=staging
IMAGE=ghcr.io/fyrst-dev/acme
IMAGE_TAG=deadbeef
COMPOSE_PROFILES=redis,setup
",
        )
        .unwrap();
        let err = execute_release(&process(&shop.0), &shop.0, false, true).unwrap_err();
        assert!(err.to_string().contains("setup"), "{err}");
    }
}
