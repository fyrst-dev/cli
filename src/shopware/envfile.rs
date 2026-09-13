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

fn is_env_key(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
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
}
