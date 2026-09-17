//! Upsert top-level Compose `name:` in shop-root `compose.override.yaml`.
//!
//! shopware-cli regenerates `compose.yaml` and leaves `compose.override.yaml`
//! for local customization. Compose (and `shopware-cli project dev`) read
//! `COMPOSE_PROJECT_NAME` only from project-directory `.env`, not `.env.local`,
//! so the YAML `name:` key is what actually names the local stack.

pub const OVERRIDE_REL: &str = "compose.override.yaml";

const EMPTY_FILE_HEADER: &str =
    "# Written by fyrst-cli shopware env init. Compose project for shopware-cli project dev.\n";

/// Set or replace the top-level `name:` scalar. Nested `name:` keys (indented)
/// and other mappings are left untouched. Returns `(contents, changed)`.
pub fn upsert_top_level_name(contents: &str, project: &str) -> (String, bool) {
    let desired = format!("name: {project}");
    if contents.trim().is_empty() {
        return (format!("{EMPTY_FILE_HEADER}{desired}\n"), true);
    }

    let mut out = String::new();
    let mut found = false;
    let mut changed = false;
    for line in contents.lines() {
        let line = trim_cr(line);
        if is_top_level_name_key(line) {
            found = true;
            if line.trim() == desired {
                out.push_str(line);
            } else {
                out.push_str(&desired);
                changed = true;
            }
            out.push('\n');
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if found {
        return (out, changed);
    }
    (insert_after_header(contents, &desired), true)
}

fn insert_after_header(contents: &str, desired: &str) -> String {
    let lines: Vec<&str> = contents.lines().map(trim_cr).collect();
    let mut insert_at = 0;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || t == "---" {
            insert_at = i + 1;
            continue;
        }
        break;
    }
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i == insert_at {
            out.push_str(desired);
            out.push('\n');
        }
        out.push_str(line);
        out.push('\n');
    }
    if insert_at >= lines.len() {
        out.push_str(desired);
        out.push('\n');
    }
    out
}

fn is_top_level_name_key(line: &str) -> bool {
    if line.starts_with(' ') || line.starts_with('\t') {
        return false;
    }
    let t = line.trim();
    if t.is_empty() || t.starts_with('#') {
        return false;
    }
    let Some(after) = t.strip_prefix("name") else {
        return false;
    };
    after.trim_start().starts_with(':')
}

fn trim_cr(line: &str) -> &str {
    line.trim_end_matches('\r')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_file_with_name() {
        let (out, changed) = upsert_top_level_name("", "acme-dev");
        assert!(changed);
        assert!(out.contains("name: acme-dev\n"));
        assert!(out.contains("fyrst-cli shopware env init"));
        assert!(!out.contains("services:"));
    }

    #[test]
    fn replaces_existing_top_level_name_keeps_services() {
        let src = "\
# keep me
name: old-folder
services:
  web:
    name: nested-must-stay
    ports:
      - \"9003:9003\"
";
        let (out, changed) = upsert_top_level_name(src, "acme-dev");
        assert!(changed);
        assert!(out.contains("name: acme-dev\n"));
        assert!(!out.contains("name: old-folder"));
        assert!(out.contains("    name: nested-must-stay"));
        assert!(out.contains("# keep me"));
        assert!(out.contains("      - \"9003:9003\""));
    }

    #[test]
    fn idempotent_when_already_set() {
        let src = "name: acme-live\nservices:\n  web:\n    image: x\n";
        let (out, changed) = upsert_top_level_name(src, "acme-live");
        assert!(!changed);
        assert_eq!(out, src);
    }

    #[test]
    fn inserts_after_document_start_when_missing() {
        let src = "\
---
services:
  web:
    image: x
";
        let (out, changed) = upsert_top_level_name(src, "acme-staging");
        assert!(changed);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "---");
        assert_eq!(lines[1], "name: acme-staging");
        assert!(out.contains("services:"));
        assert_eq!(out.matches("name:").count(), 1);
    }
}
