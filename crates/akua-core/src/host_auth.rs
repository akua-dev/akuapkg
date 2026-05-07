//! Host-keyed HTTPS auth for `git_fetcher` (and future fetchers that
//! need credentials).
//!
//! Credentials are passed at the API call site, never read from
//! ambient files / env vars. Akua's wasmtime sandbox + multi-tenant
//! consumers (e.g. cnap) require explicit auth — implicit lookups
//! would risk leaking credentials across tenants on shared hosts.
//!
//! Resolution: longest-URL-prefix match. Same algorithm git's
//! credential helper uses; same as `.npmrc` URL keys. Caller provides
//! a map keyed by URL prefix (host, or host + path prefix), the
//! fetcher looks up each fetched URL by walking entries sorted by
//! prefix length descending.
//!
//! Two helpers:
//! - [`lookup`] — finds the matching credential for a given URL.
//! - [`canonicalize_url`] — strips userinfo + default ports + `.git`
//!   so the same logical repo always produces the same string. Used
//!   for cache keys and lockfile `source` values so credentials never
//!   leak into `akua.lock` even if a malformed call ever reaches the
//!   fetcher.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// HTTP basic-auth credential pair. Sent as `Authorization: Basic
/// <base64(user:pass)>` on the underlying HTTPS request.
///
/// Bearer-token auth is intentionally not modeled here yet — git
/// transports overwhelmingly use basic auth (with the token as the
/// password), and adding a discriminated enum is non-breaking when
/// the need surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BasicAuth {
    pub username: String,
    pub password: String,
}

impl BasicAuth {
    /// `Authorization:` header value the caller hands to the HTTP
    /// transport (e.g. via gix's `http.extraHeader` config override).
    pub fn to_authorization_header(&self) -> String {
        use base64::Engine as _;
        let raw = format!("{}:{}", self.username, self.password);
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
        format!("Basic {encoded}")
    }
}

/// Caller-supplied host-keyed credentials. Keys are URL prefixes
/// without scheme — e.g. `akua-git.cnap.tech` (host only) or
/// `akua-git.cnap.tech/org-A` (host + path scope).
pub type HostAuthMap = HashMap<String, BasicAuth>;

/// Find the credential whose key is the longest prefix of `url`'s
/// scheme-less form. Returns `None` when no entry matches.
///
/// Match key is `host[:port]/path` (no scheme, no userinfo, trailing
/// `/` trimmed). Prefix entries are matched against this key, so
/// `akua-git.cnap.tech` matches `akua-git.cnap.tech/foo/bar` and
/// `akua-git.cnap.tech/org-A` matches the latter more specifically.
pub fn lookup<'a>(map: &'a HostAuthMap, url: &str) -> Option<&'a BasicAuth> {
    if map.is_empty() {
        return None;
    }
    let key = match_key(url)?;
    let mut prefixes: Vec<&str> = map.keys().map(String::as_str).collect();
    // Longest first. Same length → arbitrary; map entries shouldn't
    // overlap meaningfully.
    prefixes.sort_by_key(|s| std::cmp::Reverse(s.len()));
    for prefix in prefixes {
        if prefix_matches(&key, prefix) {
            return map.get(prefix);
        }
    }
    None
}

/// True when `prefix` is a path-aware prefix of `key`. Beyond raw
/// `starts_with`, this enforces a path-segment boundary so
/// `example.com` doesn't match `example.com.evil.com` and
/// `example.com/foo` doesn't match `example.com/foobar`.
fn prefix_matches(key: &str, prefix: &str) -> bool {
    if !key.starts_with(prefix) {
        return false;
    }
    // Exact match.
    if key.len() == prefix.len() {
        return true;
    }
    // Boundary char: prefix ends right before `/` or `:` (port).
    let next = key.as_bytes()[prefix.len()];
    next == b'/' || next == b':'
}

/// Strip scheme + userinfo from `url`, returning `host[:port]/path`
/// with no trailing `/`. Returns `None` when the URL has no host.
fn match_key(url: &str) -> Option<String> {
    let after_scheme = strip_scheme(url);
    let after_userinfo = strip_userinfo(after_scheme);
    if after_userinfo.is_empty() {
        return None;
    }
    let trimmed = after_userinfo.trim_end_matches('/');
    Some(trimmed.to_string())
}

fn strip_scheme(url: &str) -> &str {
    if let Some((_, rest)) = url.split_once("://") {
        rest
    } else {
        url
    }
}

/// Drop everything up to and including the first `@` *before* the
/// path, if any. Path-portion `@`s (e.g. `/path/with@symbol`) are
/// preserved.
fn strip_userinfo(authority_and_path: &str) -> &str {
    let path_start = authority_and_path.find('/').unwrap_or(authority_and_path.len());
    let authority = &authority_and_path[..path_start];
    if let Some(at_idx) = authority.rfind('@') {
        &authority_and_path[at_idx + 1..]
    } else {
        authority_and_path
    }
}

/// Canonicalize `url` for cache keys + lockfile storage. Idempotent.
///
/// Transformations:
/// - Strip userinfo (`https://user:pass@host/...` → `https://host/...`)
/// - Strip default ports (`:443` for `https`, `:80` for `http`)
/// - Strip trailing `.git` (when it's the last path segment)
/// - Strip trailing `/`
///
/// Returns the input unchanged on parse failure — defensive; callers
/// already validate URLs before reaching the fetcher.
pub fn canonicalize_url(url: &str) -> String {
    let Some((scheme, after_scheme)) = url.split_once("://") else {
        return url.to_string();
    };
    let after_userinfo = strip_userinfo(after_scheme);

    let (authority, path) = match after_userinfo.split_once('/') {
        Some((a, p)) => (a, format!("/{p}")),
        None => (after_userinfo, String::new()),
    };

    // Default-port stripping. Authority is `host` or `host:port`.
    let canon_authority = match authority.rsplit_once(':') {
        Some((host, port)) if is_default_port(scheme, port) => host.to_string(),
        _ => authority.to_string(),
    };

    let mut out = format!("{scheme}://{canon_authority}{path}");
    out = out.trim_end_matches('/').to_string();
    if out.ends_with(".git") {
        out.truncate(out.len() - 4);
    }
    out.trim_end_matches('/').to_string()
}

fn is_default_port(scheme: &str, port: &str) -> bool {
    matches!((scheme, port), ("https", "443") | ("http", "80"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basic(user: &str, pass: &str) -> BasicAuth {
        BasicAuth {
            username: user.to_string(),
            password: pass.to_string(),
        }
    }

    #[test]
    fn lookup_empty_map_returns_none() {
        let map: HostAuthMap = HashMap::new();
        assert!(lookup(&map, "https://github.com/foo/bar").is_none());
    }

    #[test]
    fn lookup_host_prefix_matches() {
        let mut map: HostAuthMap = HashMap::new();
        map.insert("github.com".into(), basic("alice", "tok1"));
        let creds = lookup(&map, "https://github.com/foo/bar").unwrap();
        assert_eq!(creds, &basic("alice", "tok1"));
    }

    #[test]
    fn lookup_longest_prefix_wins() {
        let mut map: HostAuthMap = HashMap::new();
        map.insert("akua-git.cnap.tech".into(), basic("svc", "fallback"));
        map.insert("akua-git.cnap.tech/org-A".into(), basic("orgA", "tokA"));
        map.insert("akua-git.cnap.tech/org-B".into(), basic("orgB", "tokB"));

        assert_eq!(
            lookup(&map, "https://akua-git.cnap.tech/org-A/repo.git").unwrap(),
            &basic("orgA", "tokA")
        );
        assert_eq!(
            lookup(&map, "https://akua-git.cnap.tech/org-B/repo.git").unwrap(),
            &basic("orgB", "tokB")
        );
        // Falls back to host-level entry.
        assert_eq!(
            lookup(&map, "https://akua-git.cnap.tech/other/repo.git").unwrap(),
            &basic("svc", "fallback")
        );
    }

    #[test]
    fn lookup_respects_path_segment_boundary() {
        // `example.com/org` must NOT match `example.com/organizations`.
        let mut map: HostAuthMap = HashMap::new();
        map.insert("example.com/org".into(), basic("u", "p"));
        assert!(lookup(&map, "https://example.com/organizations/x").is_none());
        assert!(lookup(&map, "https://example.com/org/x").is_some());
    }

    #[test]
    fn lookup_ignores_userinfo_in_url() {
        let mut map: HostAuthMap = HashMap::new();
        map.insert("github.com".into(), basic("alice", "tok"));
        // Even if a malformed URL slips userinfo in, the lookup key
        // stripping means the host still matches cleanly.
        assert!(lookup(&map, "https://attacker:tok@github.com/foo").is_some());
    }

    #[test]
    fn lookup_no_host_match_returns_none() {
        let mut map: HostAuthMap = HashMap::new();
        map.insert("github.com".into(), basic("u", "p"));
        assert!(lookup(&map, "https://gitlab.com/foo").is_none());
    }

    #[test]
    fn lookup_with_explicit_port() {
        let mut map: HostAuthMap = HashMap::new();
        map.insert("localhost:3000".into(), basic("admin", "secret"));
        assert!(lookup(&map, "http://localhost:3000/foo/bar").is_some());
        // Different port — no match.
        assert!(lookup(&map, "http://localhost:4000/foo/bar").is_none());
    }

    #[test]
    fn canonicalize_strips_userinfo() {
        assert_eq!(
            canonicalize_url("https://user:tok@github.com/foo/bar"),
            "https://github.com/foo/bar"
        );
    }

    #[test]
    fn canonicalize_strips_default_ports() {
        assert_eq!(
            canonicalize_url("https://github.com:443/foo"),
            "https://github.com/foo"
        );
        assert_eq!(
            canonicalize_url("http://example.com:80/foo"),
            "http://example.com/foo"
        );
    }

    #[test]
    fn canonicalize_keeps_non_default_ports() {
        assert_eq!(
            canonicalize_url("http://localhost:3000/foo"),
            "http://localhost:3000/foo"
        );
        assert_eq!(
            canonicalize_url("https://gitlab.example.com:8443/foo"),
            "https://gitlab.example.com:8443/foo"
        );
    }

    #[test]
    fn canonicalize_strips_dot_git_and_trailing_slash() {
        assert_eq!(
            canonicalize_url("https://github.com/foo/bar.git"),
            "https://github.com/foo/bar"
        );
        assert_eq!(
            canonicalize_url("https://github.com/foo/bar/"),
            "https://github.com/foo/bar"
        );
        assert_eq!(
            canonicalize_url("https://github.com/foo/bar.git/"),
            "https://github.com/foo/bar"
        );
    }

    #[test]
    fn canonicalize_is_idempotent() {
        let canon = canonicalize_url("https://user:tok@github.com:443/foo/bar.git/");
        assert_eq!(canon, canonicalize_url(&canon));
        assert_eq!(canon, "https://github.com/foo/bar");
    }

    #[test]
    fn canonicalize_preserves_unparseable_input() {
        // No `://` → return as-is rather than corrupting.
        assert_eq!(canonicalize_url("not-a-url"), "not-a-url");
    }
}
