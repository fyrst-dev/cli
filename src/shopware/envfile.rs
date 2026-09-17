//! KEY=VALUE `.env` parsing (no shell expansion).
//!
//! Recipe scripts `source` `.env` in bash, which expands `$…`. This CLI treats
//! values as literals so passwords containing `$` stay intact. Quotes are
//! stripped; double-quoted `\\`, `\\n`, and `\\t` are unescaped.

pub fn parse_env_file(contents: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw in contents.lines() {
        let mut line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("export ") {
            line = rest.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if !is_env_key(key) {
            continue;
        }
        out.push((key.to_string(), unquote(val.trim())));
    }
    out
}

/// Last uncommented assignment wins (Compose / Docker dotenv). Missing key → empty.
pub fn last_value(contents: &str, key: &str) -> String {
    parse_env_file(contents)
        .into_iter()
        .filter(|(k, _)| k == key)
        .map(|(_, v)| v)
        .last()
        .unwrap_or_default()
}

/// Uncommented `KEY=` assignment is present (value may be empty).
pub fn has_key(contents: &str, key: &str) -> bool {
    parse_env_file(contents).iter().any(|(k, _)| k == key)
}

pub const MERGE_FROM_EXAMPLE_HEADER: &str =
    "# --- missing keys merged from .env.example by deploy/init-env.sh ---";

/// Keys that must not be copied from `.env.example` into committed `.env`.
const SKIP_MERGE_INTO_SHARED_ENV: &[&str] = &["COMPOSE_PROJECT_NAME", "SHOPWARE_DEPLOY_ENV"];

pub const COMPOSE_PROJECT_NAME_COMMENT_SUFFIX: &str =
    " # commented by fyrst-cli shopware env init (Compose project is ${SHOPWARE_SHOP_ID}-${SHOPWARE_DEPLOY_ENV} via docker compose -p)";

pub const SHOPWARE_DEPLOY_ENV_COMMENT_SUFFIX: &str =
    " # commented by fyrst-cli shopware env init (host-specific; set in .env.local)";

/// Replace every uncommented `KEY=` line, or append `KEY=value`.
/// Preserves an `export` prefix. Does not quote `value` (overlay `env_set_key`).
pub fn set_key(contents: &str, key: &str, value: &str) -> String {
    let mut out = String::new();
    let mut found = false;
    for line in contents.lines() {
        let line = trim_cr(line);
        if let Some(export) = uncommented_assignment(line, key) {
            if export {
                out.push_str("export ");
            }
            out.push_str(key);
            out.push('=');
            out.push_str(value);
            out.push('\n');
            found = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !found {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(key);
        out.push('=');
        out.push_str(value);
        out.push('\n');
    }
    out
}

/// Comment uncommented `KEY=` lines. Returns the rewritten file and how many
/// lines were commented. Already-commented lines are left as-is.
pub fn comment_uncommented_key(contents: &str, key: &str, suffix: &str) -> (String, usize) {
    let mut out = String::new();
    let mut n = 0;
    for line in contents.lines() {
        let line = trim_cr(line);
        if uncommented_assignment(line, key).is_some() {
            out.push_str("# ");
            out.push_str(line);
            out.push_str(suffix);
            out.push('\n');
            n += 1;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    (out, n)
}

pub fn comment_compose_project_name(contents: &str) -> (String, usize) {
    comment_uncommented_key(
        contents,
        "COMPOSE_PROJECT_NAME",
        COMPOSE_PROJECT_NAME_COMMENT_SUFFIX,
    )
}

pub fn comment_deploy_env(contents: &str) -> (String, usize) {
    comment_uncommented_key(
        contents,
        "SHOPWARE_DEPLOY_ENV",
        SHOPWARE_DEPLOY_ENV_COMMENT_SUFFIX,
    )
}

/// Append assignment lines from `.env.example` whose keys are missing in dest.
/// Skips `COMPOSE_PROJECT_NAME` and `SHOPWARE_DEPLOY_ENV` (not for shared `.env`).
pub fn merge_missing_from_example(example: &str, dest: &mut String) -> Vec<String> {
    let mut added = Vec::new();
    let mut header = false;
    for line in example.lines() {
        let line = trim_cr(line);
        if is_comment_or_blank(line) {
            continue;
        }
        let Some(key) = uncommented_key(line) else {
            continue;
        };
        if SKIP_MERGE_INTO_SHARED_ENV.contains(&key) {
            continue;
        }
        if has_key(dest, key) {
            continue;
        }
        if !header {
            dest.push('\n');
            dest.push_str(MERGE_FROM_EXAMPLE_HEADER);
            dest.push('\n');
            header = true;
        }
        dest.push_str(line);
        dest.push('\n');
        added.push(key.to_string());
    }
    added
}

fn is_env_key(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn trim_cr(line: &str) -> &str {
    line.trim_end_matches('\r')
}

fn is_comment_or_blank(line: &str) -> bool {
    let t = line.trim_start();
    t.is_empty() || t.starts_with('#')
}

fn strip_export(line: &str) -> (bool, &str) {
    let rest = line.trim_start();
    match rest.strip_prefix("export") {
        Some(after) if after.chars().next().is_some_and(|c| c.is_whitespace()) => {
            (true, after.trim_start())
        }
        _ => (false, rest),
    }
}

/// Overlay `^[[:space:]]*(export[[:space:]]+)?KEY=`. `Some(export)`.
fn uncommented_assignment(line: &str, key: &str) -> Option<bool> {
    if is_comment_or_blank(line) {
        return None;
    }
    let (export, rest) = strip_export(line);
    rest.strip_prefix(key)
        .and_then(|tail| tail.starts_with('=').then_some(export))
}

fn uncommented_key(line: &str) -> Option<&str> {
    if is_comment_or_blank(line) {
        return None;
    }
    let (_, rest) = strip_export(line);
    let (key, _) = rest.split_once('=')?;
    if key.is_empty() || key.chars().any(char::is_whitespace) || !is_env_key(key) {
        return None;
    }
    Some(key)
}

fn unquote(s: &str) -> String {
    let b = s.as_bytes();
    if b.len() >= 2 && b[0] == b'"' && b[b.len() - 1] == b'"' {
        return unescape_double(&s[1..s.len() - 1]);
    }
    if b.len() >= 2 && b[0] == b'\'' && b[b.len() - 1] == b'\'' {
        return s[1..s.len() - 1].to_string();
    }
    s.to_string()
}

fn unescape_double(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Percent-decode like Python `urllib.parse.unquote` (`+` stays `+`).
pub fn urldecode(input: &str) -> String {
    let b = input.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h1), Some(h2)) = (from_hex(b[i + 1]), from_hex(b[i + 2])) {
                out.push((h1 << 4) | h2);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_comments_and_strips_quotes() {
        let parsed = parse_env_file(
            "\
# comment
SHOPWARE_SHOP_ID=acme
export MYSQL_USER=shop
MYSQL_PASSWORD=\"p@ss word\"
EMPTY=
NOT_A_LINE
",
        );
        assert_eq!(
            parsed,
            vec![
                ("SHOPWARE_SHOP_ID".into(), "acme".into()),
                ("MYSQL_USER".into(), "shop".into()),
                ("MYSQL_PASSWORD".into(), "p@ss word".into()),
                ("EMPTY".into(), "".into()),
            ]
        );
    }

    #[test]
    fn does_not_expand_dollar() {
        let parsed = parse_env_file("MYSQL_PASSWORD=pa$$word\n");
        assert_eq!(parsed[0].1, "pa$$word");
    }

    #[test]
    fn urldecode_percent_at() {
        assert_eq!(urldecode("p%40ss"), "p@ss");
        assert_eq!(urldecode("a+b"), "a+b");
        assert_eq!(urldecode("a%20b"), "a b");
    }

    #[test]
    fn last_value_wins_and_has_key() {
        let src = "\
SHOPWARE_SHOP_ID=first
SHOPWARE_SHOP_ID=acme
# APP_SECRET=commented
APP_SECRET=
";
        assert_eq!(last_value(src, "SHOPWARE_SHOP_ID"), "acme");
        assert!(has_key(src, "APP_SECRET"));
        assert_eq!(last_value(src, "APP_SECRET"), "");
        assert!(!has_key(src, "IMAGE"));
        assert_eq!(last_value(src, "IMAGE"), "");
    }

    #[test]
    fn set_key_replaces_all_and_keeps_export() {
        let src = "\
# keep
export IMAGE=old
IMAGE=also-old
MYSQL_PASSWORD=pa$$word
";
        let out = set_key(src, "IMAGE", "ghcr.io/example/acme");
        assert_eq!(
            out,
            "\
# keep
export IMAGE=ghcr.io/example/acme
IMAGE=ghcr.io/example/acme
MYSQL_PASSWORD=pa$$word
"
        );
        assert!(out.contains("pa$$word"));
        let appended = set_key("FOO=1\n", "SHOPWARE_SHOP_ID", "acme");
        assert_eq!(appended, "FOO=1\nSHOPWARE_SHOP_ID=acme\n");
    }

    #[test]
    fn comment_uncommented_compose_project_name_skips_already_commented() {
        let src = "\
COMPOSE_PROJECT_NAME=sw-shop-acme
export COMPOSE_PROJECT_NAME=other
# COMPOSE_PROJECT_NAME=keep-commented
MYSQL_PASSWORD=s3cret
";
        let (out, n) = comment_compose_project_name(src);
        assert_eq!(n, 2);
        assert!(!has_key(&out, "COMPOSE_PROJECT_NAME"));
        assert!(!out
            .lines()
            .any(|l| l.trim_start().starts_with("COMPOSE_PROJECT_NAME=")));
        assert!(out.contains(
            "# COMPOSE_PROJECT_NAME=sw-shop-acme # commented by fyrst-cli shopware env init"
        ));
        assert!(out.contains(
            "# export COMPOSE_PROJECT_NAME=other # commented by fyrst-cli shopware env init"
        ));
        assert!(out.contains("# COMPOSE_PROJECT_NAME=keep-commented"));
        assert!(out.contains("MYSQL_PASSWORD=s3cret"));

        let (again, n2) = comment_compose_project_name(&out);
        assert_eq!(n2, 0);
        assert_eq!(again, out);
    }

    #[test]
    fn comment_uncommented_deploy_env_leaves_commented_lines() {
        let src = "\
SHOPWARE_DEPLOY_ENV=staging
export SHOPWARE_DEPLOY_ENV=live
# SHOPWARE_DEPLOY_ENV=dev
SHOPWARE_SHOP_ID=acme
";
        let (out, n) = comment_deploy_env(src);
        assert_eq!(n, 2);
        assert!(!has_key(&out, "SHOPWARE_DEPLOY_ENV"));
        assert!(out
            .contains("# SHOPWARE_DEPLOY_ENV=staging # commented by fyrst-cli shopware env init"));
        assert!(out.contains(
            "# export SHOPWARE_DEPLOY_ENV=live # commented by fyrst-cli shopware env init"
        ));
        assert!(out.contains("# SHOPWARE_DEPLOY_ENV=dev"));
        assert_eq!(last_value(&out, "SHOPWARE_SHOP_ID"), "acme");
    }

    #[test]
    fn merge_missing_copies_lines_without_clobber() {
        let example = "\
# comment
SHOPWARE_SHOP_ID=
MYSQL_PASSWORD=from-example
APP_URL=http://localhost
IMAGE=from-example
";
        let mut dest = "\
SHOPWARE_SHOP_ID=
MYSQL_PASSWORD=keep-me
"
        .to_string();
        let added = merge_missing_from_example(example, &mut dest);
        assert_eq!(added, vec!["APP_URL", "IMAGE"]);
        assert!(dest.contains("MYSQL_PASSWORD=keep-me"));
        assert!(!dest.contains("MYSQL_PASSWORD=from-example"));
        assert!(dest.contains(MERGE_FROM_EXAMPLE_HEADER));
        assert!(dest.contains("APP_URL=http://localhost"));
        assert!(dest.contains("IMAGE=from-example"));
        assert_eq!(last_value(&dest, "SHOPWARE_SHOP_ID"), "");
    }

    #[test]
    fn merge_missing_skips_compose_project_name_and_deploy_env() {
        let example = "\
SHOPWARE_SHOP_ID=
COMPOSE_PROJECT_NAME=sw-shop-acme
SHOPWARE_DEPLOY_ENV=live
APP_URL=http://localhost
";
        let mut dest = "SHOPWARE_SHOP_ID=\n".to_string();
        let added = merge_missing_from_example(example, &mut dest);
        assert_eq!(added, vec!["APP_URL"]);
        assert!(!has_key(&dest, "COMPOSE_PROJECT_NAME"));
        assert!(!has_key(&dest, "SHOPWARE_DEPLOY_ENV"));
        assert!(dest.contains("APP_URL=http://localhost"));
    }
}
