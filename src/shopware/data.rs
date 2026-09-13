//! `--data` selection (same items as `deploy/lib/sync-commands.sh`).

use super::error::Error;

pub const DEFAULT_DATA: &str = "db,media,files,thumbnail,theme,sitemap";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataSelection {
    pub want_db: bool,
    pub volumes: Vec<String>,
}

impl DataSelection {
    pub fn items_csv(&self) -> String {
        let mut v = Vec::new();
        if self.want_db {
            v.push("db".to_string());
        }
        v.extend(self.volumes.iter().cloned());
        v.join(",")
    }
}

pub fn normalize_data(
    spec: Option<&str>,
    skip_db: bool,
    skip_volumes: bool,
) -> Result<DataSelection, Error> {
    let mut spec = spec.unwrap_or("all").trim().to_string();
    if spec.is_empty() {
        spec = "all".into();
    }
    if spec.eq_ignore_ascii_case("all") {
        spec = DEFAULT_DATA.to_string();
    }

    let mut want_db = false;
    let mut volumes = Vec::new();
    let mut saw_item = false;
    for item in spec.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        saw_item = true;
        let lower = item.to_ascii_lowercase();
        match lower.as_str() {
            "db" | "database" | "mysql" => want_db = true,
            "files" | "media" | "thumbnail" | "theme" | "sitemap" => {
                if !volumes.iter().any(|v| v == &lower) {
                    volumes.push(lower);
                }
            }
            "mysql_data" | "redis_data" => {
                return Err(Error::fail(format!(
                    "Refusing volume '{lower}'. Copy the database with --data db (SQL dump), not the {lower} volume."
                )));
            }
            _ => {
                return Err(Error::fail(format!(
                    "Unknown --data item '{item}'. Use db, files, media, thumbnail, theme, sitemap, or all."
                )));
            }
        }
    }
    if !saw_item {
        return Err(Error::fail("--data is empty"));
    }
    if skip_db {
        want_db = false;
    }
    if skip_volumes {
        volumes.clear();
    }
    if !want_db && volumes.is_empty() {
        return Err(Error::fail(
            "Nothing to do (--data plus --skip-db/--skip-volumes selected an empty set)",
        ));
    }
    Ok(DataSelection { want_db, volumes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_all_includes_db_and_volumes() {
        let d = normalize_data(None, false, false).unwrap();
        assert!(d.want_db);
        assert_eq!(
            d.volumes,
            vec!["media", "files", "thumbnail", "theme", "sitemap"]
        );
    }

    #[test]
    fn skip_volumes_leaves_db() {
        let d = normalize_data(Some("all"), false, true).unwrap();
        assert!(d.want_db);
        assert!(d.volumes.is_empty());
    }

    #[test]
    fn skip_db_on_default_leaves_volumes() {
        let d = normalize_data(Some("all"), true, false).unwrap();
        assert!(!d.want_db);
        assert!(!d.volumes.is_empty());
    }

    #[test]
    fn aliases_and_csv_spaces() {
        let d = normalize_data(Some("database, media"), false, false).unwrap();
        assert!(d.want_db);
        assert_eq!(d.volumes, vec!["media"]);
    }

    #[test]
    fn empty_after_skips_fails() {
        let err = normalize_data(Some("db"), true, false).unwrap_err();
        assert!(err.to_string().contains("Nothing to do"), "{err}");
    }

    #[test]
    fn refuses_mysql_data_volume() {
        let err = normalize_data(Some("mysql_data"), false, false).unwrap_err();
        assert!(err.to_string().contains("mysql_data"), "{err}");
    }

    #[test]
    fn unknown_item_fails() {
        let err = normalize_data(Some("logs"), false, false).unwrap_err();
        assert!(err.to_string().contains("Unknown"), "{err}");
    }
}
