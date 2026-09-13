//! `fyrst-cli shopware sync snapshot` — not a dump wrapper.
//!
//! Database dumps are owned by `shopware-cli project dump`. This command does
//! not dump, wrap, or shell out to shopware-cli. Bind-mount volume copy is
//! still stub.

use super::data::normalize_data;
use super::env::is_local_source;
use super::error::Error;
use crate::cli::SyncOpArgs;

pub fn run(args: SyncOpArgs) -> Result<(), Error> {
    evaluate(&args)
}

pub fn evaluate(args: &SyncOpArgs) -> Result<(), Error> {
    let selection = normalize_data(args.data.as_deref(), args.skip_db, args.skip_volumes)?;
    if !is_local_source(args.from.as_deref()) {
        let alias = args.from.as_deref().unwrap_or("").trim();
        return Err(Error::stub(format!(
            "shopware sync snapshot --from {alias} (remote SSH is not implemented yet; use --from local). Database dumps are owned by shopware-cli (`shopware-cli project dump`); fyrst-cli does not dump databases."
        )));
    }
    if selection.want_db {
        let mut msg = String::from(
            "database dump. Dumps are owned by shopware-cli (`shopware-cli project dump`). fyrst-cli does not dump databases and does not wrap or shell out to shopware-cli. Import a dump with: fyrst-cli shopware db import --file <path.sql|.sql.gz>",
        );
        if !selection.volumes.is_empty() {
            msg.push_str(&format!(
                " Bind-mount volume snapshot ({}) is also not implemented.",
                selection.volumes.join(", ")
            ));
        }
        return Err(Error::stub(msg));
    }
    Err(Error::stub(format!(
        "bind-mount volume snapshot ({})",
        selection.volumes.join(", ")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::SyncOpArgs;

    fn args(data: Option<&str>, from: Option<&str>) -> SyncOpArgs {
        SyncOpArgs {
            from: from.map(str::to_string),
            data: data.map(str::to_string),
            snapshot_dir: None,
            dry_run: true,
            skip_db: false,
            skip_volumes: false,
        }
    }

    #[test]
    fn db_points_at_shopware_cli() {
        let err = evaluate(&args(Some("db"), Some("local"))).unwrap_err();
        match err {
            Error::NotImplemented(m) => {
                assert!(m.contains("shopware-cli project dump"), "{m}");
                assert!(m.contains("db import"), "{m}");
                assert!(!m.contains("docker run"), "{m}");
            }
            Error::Fail(m) => panic!("expected stub: {m}"),
        }
    }

    #[test]
    fn default_data_mentions_volumes_too() {
        let err = evaluate(&args(None, None)).unwrap_err();
        match err {
            Error::NotImplemented(m) => {
                assert!(m.contains("shopware-cli"), "{m}");
                assert!(m.contains("volume"), "{m}");
            }
            Error::Fail(m) => panic!("expected stub: {m}"),
        }
    }

    #[test]
    fn volumes_only_is_stub() {
        let err = evaluate(&args(Some("media"), None)).unwrap_err();
        match err {
            Error::NotImplemented(m) => assert!(m.contains("volume"), "{m}"),
            Error::Fail(m) => panic!("expected stub: {m}"),
        }
    }

    #[test]
    fn remote_from_is_stub() {
        let err = evaluate(&args(Some("db"), Some("live"))).unwrap_err();
        match err {
            Error::NotImplemented(m) => {
                assert!(m.contains("remote SSH"), "{m}");
                assert!(m.contains("live"), "{m}");
            }
            Error::Fail(m) => panic!("expected stub: {m}"),
        }
    }
}
