//! Parsing for `--auth` and `--auth-file` flags. Builds a
//! [`HostAuthMap`] that's threaded down to the akua-core verbs.
//!
//! Akua never reads ambient credential files — this module only
//! consumes user-explicit input (CLI flags or a path the user named
//! via `--auth-file`).

use std::collections::HashMap;
use std::path::Path;

use akua_core::host_auth::{BasicAuth, HostAuthMap};

/// Errors `parse_auth_pair` and `load_auth_file` can produce. Each
/// variant carries enough detail for the CLI to print a useful
/// stderr message including the offending input.
#[derive(Debug, thiserror::Error)]
pub enum AuthParseError {
    #[error(
        "--auth value `{value}` is not in the form `<host-or-prefix>=<user>:<password>`"
    )]
    InvalidPair { value: String },

    #[error("--auth value `{value}` has an empty url-prefix on the left of `=`")]
    EmptyPrefix { value: String },

    #[error("--auth value `{value}` has an empty username on the left of `:` after `=`")]
    EmptyUsername { value: String },

    #[error("could not read --auth-file `{path}`: {source}")]
    ReadFile {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("could not parse --auth-file `{path}` as TOML: {source}")]
    ParseFile {
        path: String,
        #[source]
        source: toml::de::Error,
    },
}

/// Parse one `<prefix>=<username>:<password>` value (a single
/// `--auth` flag occurrence). The split is "first `=`" then "first
/// `:`" so passwords containing `:` or `=` work without escaping.
pub fn parse_auth_pair(value: &str) -> Result<(String, BasicAuth), AuthParseError> {
    let (prefix, rest) = value.split_once('=').ok_or_else(|| AuthParseError::InvalidPair {
        value: value.to_string(),
    })?;
    if prefix.is_empty() {
        return Err(AuthParseError::EmptyPrefix {
            value: value.to_string(),
        });
    }
    let (username, password) = rest.split_once(':').ok_or_else(|| AuthParseError::InvalidPair {
        value: value.to_string(),
    })?;
    if username.is_empty() {
        return Err(AuthParseError::EmptyUsername {
            value: value.to_string(),
        });
    }
    Ok((
        prefix.to_string(),
        BasicAuth {
            username: username.to_string(),
            password: password.to_string(),
        },
    ))
}

/// Build a [`HostAuthMap`] from the repeatable `--auth` flag values.
/// Returns `Ok(None)` for an empty input — the caller treats that
/// as "no credentials" rather than an empty map.
pub fn parse_auth_pairs<I, S>(values: I) -> Result<Option<HostAuthMap>, AuthParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut map: HostAuthMap = HashMap::new();
    for v in values {
        let (prefix, creds) = parse_auth_pair(v.as_ref())?;
        map.insert(prefix, creds);
    }
    if map.is_empty() {
        Ok(None)
    } else {
        Ok(Some(map))
    }
}

/// File shape:
///
/// ```toml
/// [auth]
/// "akua-git.cnap.tech/org-A" = { username = "...", password = "..." }
/// "github.com" = { username = "...", password = "..." }
/// ```
///
/// Top-level `[auth]` table keyed by URL prefix; values are
/// `{ username, password }` records. Quotes around the prefix keys
/// are required by TOML when the key contains `/`.
pub fn load_auth_file(path: &Path) -> Result<HostAuthMap, AuthParseError> {
    let text = std::fs::read_to_string(path).map_err(|e| AuthParseError::ReadFile {
        path: path.display().to_string(),
        source: e,
    })?;
    let parsed: AuthFile = toml::from_str(&text).map_err(|e| AuthParseError::ParseFile {
        path: path.display().to_string(),
        source: e,
    })?;
    Ok(parsed.auth)
}

/// Merge `--auth-file` defaults with `--auth` overrides. CLI flag
/// values win on conflict (last-set semantics, same as helm/docker).
pub fn merge_auth(
    file: Option<HostAuthMap>,
    flags: Option<HostAuthMap>,
) -> Option<HostAuthMap> {
    match (file, flags) {
        (None, None) => None,
        (Some(f), None) => Some(f),
        (None, Some(f)) => Some(f),
        (Some(mut base), Some(over)) => {
            for (k, v) in over {
                base.insert(k, v);
            }
            Some(base)
        }
    }
}

#[derive(serde::Deserialize)]
struct AuthFile {
    #[serde(default)]
    auth: HostAuthMap,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_pair() {
        let (prefix, creds) = parse_auth_pair("github.com=alice:tok123").unwrap();
        assert_eq!(prefix, "github.com");
        assert_eq!(creds.username, "alice");
        assert_eq!(creds.password, "tok123");
    }

    #[test]
    fn parse_password_with_colon() {
        let (_, creds) = parse_auth_pair("host=user:pass:with:colons").unwrap();
        assert_eq!(creds.password, "pass:with:colons");
    }

    #[test]
    fn parse_empty_prefix_rejected() {
        assert!(matches!(
            parse_auth_pair("=user:pass"),
            Err(AuthParseError::EmptyPrefix { .. })
        ));
    }

    #[test]
    fn parse_empty_username_rejected() {
        assert!(matches!(
            parse_auth_pair("host=:pass"),
            Err(AuthParseError::EmptyUsername { .. })
        ));
    }

    #[test]
    fn parse_no_equals_rejected() {
        assert!(matches!(
            parse_auth_pair("garbage"),
            Err(AuthParseError::InvalidPair { .. })
        ));
    }

    #[test]
    fn parse_no_colon_rejected() {
        assert!(matches!(
            parse_auth_pair("host=usernoseparator"),
            Err(AuthParseError::InvalidPair { .. })
        ));
    }

    #[test]
    fn parse_pairs_returns_none_on_empty() {
        let none: Option<&str> = None;
        assert!(parse_auth_pairs(none).unwrap().is_none());
    }

    #[test]
    fn parse_pairs_collects_and_dedupes() {
        let map = parse_auth_pairs([
            "host-a=u1:p1",
            "host-b/org=u2:p2",
            "host-a=u1b:p1b", // last write wins
        ])
        .unwrap()
        .unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map["host-a"].username, "u1b");
    }

    #[test]
    fn load_auth_file_basic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.toml");
        std::fs::write(
            &path,
            r#"
[auth]
"akua-git.cnap.tech" = { username = "svc", password = "tok" }
"akua-git.cnap.tech/org-A" = { username = "orgA", password = "tokA" }
"#,
        )
        .unwrap();
        let map = load_auth_file(&path).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map["akua-git.cnap.tech"].username, "svc");
        assert_eq!(map["akua-git.cnap.tech/org-A"].password, "tokA");
    }

    #[test]
    fn merge_prefers_flags_over_file() {
        let file = parse_auth_pairs(["host=fileuser:filepass"]).unwrap();
        let flags = parse_auth_pairs(["host=flaguser:flagpass"]).unwrap();
        let merged = merge_auth(file, flags).unwrap();
        assert_eq!(merged["host"].username, "flaguser");
    }
}
