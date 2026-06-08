//! Shared HTTP + auth plumbing for OCI fetch + push.
//!
//! Kept in a dedicated module so `oci_fetcher` (GET paths for chart
//! pulls) and `oci_pusher` (POST/PUT paths for `akua publish`) don't
//! duplicate the bearer-challenge dance, token cache, and ref
//! parser. Anything OCI-spec-level ("how do you talk to a
//! distribution-API registry") lives here; anything akua-specific
//! ("what media types are helm charts" / "which layer is the
//! signature") stays in the caller.

use std::io::Read;
use std::time::Duration;

use serde::Deserialize;

use crate::oci_auth::Credentials;

/// Hard ceiling for an OCI manifest body. Manifests are small JSON
/// documents (a handful of layer descriptors); 32 MiB is orders of
/// magnitude beyond any legitimate manifest and still bounds a
/// malicious registry that streams an endless body into our heap.
pub(crate) const MAX_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;

/// Hard ceiling for a single OCI blob (chart / package tarball). Real
/// Helm charts + KCL packages are sub-MiB; 1 GiB leaves headroom for
/// pathological-but-honest artifacts while refusing a registry that
/// would otherwise stream until the host runs out of memory. The blob
/// digest is only known *after* the full body is read, so this is the
/// only thing standing between a digest-mismatch attacker and OOM.
pub(crate) const MAX_BLOB_BYTES: u64 = 1024 * 1024 * 1024;

/// Diagnostic error bodies are truncated to this many characters. Kept
/// as a char count (not a byte count) so truncation never splits a
/// multibyte UTF-8 sequence.
const MAX_ERROR_BODY_CHARS: usize = 300;

/// Parsed OCI reference. `oci://<registry>/<repo>` → the tuple. Tests
/// cover this below so non-network parser changes don't regress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OciRef {
    pub registry: String,
    pub repository: String,
}

/// HTTP scheme to use when talking to `registry`. `https` everywhere
/// except loopback hosts (`localhost`, `127.0.0.1`, `[::1]`) — matches
/// the convention `docker`, `oras`, `crane`, and `skopeo` use for
/// self-hosted local / dev / test registries. The match is on the
/// hostname *and* an optional `:port`, so `localhost:5000` and
/// `127.0.0.1:8443` both qualify.
pub(crate) fn registry_scheme(registry: &str) -> &'static str {
    // Strip an optional `:<port>` to get the bare host. IPv6 literals
    // are bracketed (`[::1]:5000`) so a trailing port is the only colon
    // *outside* the brackets; for bare-IPv6 (`::1`) we match the whole
    // string against the loopback set below.
    let host = if let Some(rest) = registry.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else if registry.matches(':').count() == 1 {
        registry.split(':').next().unwrap_or(registry)
    } else {
        registry
    };
    if matches!(host, "localhost" | "127.0.0.1" | "::1") {
        "http"
    } else {
        "https"
    }
}

/// Parse `oci://<registry>/<path/to/repo>` → `OciRef`. Scheme is
/// required — bare registry refs are an ambiguity the spec forbids.
pub(crate) fn parse_ref(s: &str) -> Result<OciRef, TransportError> {
    let rest = s
        .strip_prefix("oci://")
        .ok_or_else(|| TransportError::BadRef(s.to_string()))?;
    let (registry, repository) = rest
        .split_once('/')
        .ok_or_else(|| TransportError::BadRef(s.to_string()))?;
    if registry.is_empty() || repository.is_empty() {
        return Err(TransportError::BadRef(s.to_string()));
    }
    Ok(OciRef {
        registry: registry.to_string(),
        repository: repository.to_string(),
    })
}

/// Build a reqwest blocking client. Single place so all OCI calls
/// share a user-agent + timeout policy.
pub(crate) fn build_client() -> Result<reqwest::blocking::Client, TransportError> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(concat!("akua/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|source| TransportError::Http {
            url: "<client-construction>".to_string(),
            source,
        })
}

/// Bearer-token cache scoped to a single OCI operation. Keeps the
/// first challenge-traded token hot for subsequent manifest + blob
/// requests that share a scope.
#[derive(Default)]
pub(crate) struct TokenCache {
    pub token: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("invalid OCI reference `{0}`: expected `oci://<registry>/<repo>`")]
    BadRef(String),

    #[error("http error on `{url}`: {source}")]
    Http {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("registry returned {status} for `{url}`: {body}")]
    Status {
        url: String,
        status: u16,
        body: String,
    },

    #[error("registry `{registry}` rejected auth. Configure credentials in `~/.config/akua/auth.toml` or `docker login` for `~/.docker/config.json`.")]
    AuthRequired { registry: String },

    #[error("response body for `{url}` exceeds the {limit}-byte ceiling{}", match declared { Some(n) => format!(" (registry declared {n} bytes)"), None => String::new() })]
    ResponseTooLarge {
        url: String,
        limit: u64,
        /// `Content-Length` (or layer-declared `size`) when the
        /// oversize was caught before streaming; `None` when the body
        /// overran the cap mid-stream with no/​bogus declared length.
        declared: Option<u64>,
    },

    #[error("reading response body from `{url}`: {source}")]
    BodyRead {
        url: String,
        #[source]
        source: std::io::Error,
    },
}

/// Parsed `WWW-Authenticate: Bearer realm=...,service=...,scope=...`
/// challenge. Registries use quoted values per RFC 7235.
#[derive(Debug)]
pub(crate) struct BearerChallenge {
    pub realm: String,
    pub service: Option<String>,
    pub scope: Option<String>,
}

impl BearerChallenge {
    pub(crate) fn from_resp(resp: &reqwest::blocking::Response) -> Option<Self> {
        let hdr = resp.headers().get("WWW-Authenticate")?.to_str().ok()?;
        let rest = hdr.strip_prefix("Bearer ")?;
        let mut out = BearerChallenge {
            realm: String::new(),
            service: None,
            scope: None,
        };
        for part in rest.split(',') {
            let (k, v) = part.trim().split_once('=')?;
            let v = v.trim().trim_matches('"').to_string();
            match k.trim() {
                "realm" => out.realm = v,
                "service" => out.service = Some(v),
                "scope" => out.scope = Some(v),
                _ => {}
            }
        }
        if out.realm.is_empty() {
            return None;
        }
        Some(out)
    }
}

/// Exchange a bearer challenge for an access token. When `creds` is
/// `Some` the auth header is attached to the realm request (Basic
/// for username/password, Bearer for a raw PAT). Anonymous omits the
/// header and gets a public-scope token.
pub(crate) fn fetch_token(
    client: &reqwest::blocking::Client,
    challenge: &BearerChallenge,
    creds: Option<&Credentials>,
) -> Result<String, TransportError> {
    let mut req = client.get(&challenge.realm);
    if let Some(c) = creds {
        req = req.header("Authorization", c.to_authorization_header());
    }
    let mut query: Vec<(&str, &str)> = Vec::new();
    if let Some(service) = &challenge.service {
        query.push(("service", service));
    }
    if let Some(scope) = &challenge.scope {
        query.push(("scope", scope));
    }
    if !query.is_empty() {
        req = req.query(&query);
    }
    let resp = req.send().map_err(|source| TransportError::Http {
        url: challenge.realm.clone(),
        source,
    })?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(TransportError::Status {
            url: challenge.realm.clone(),
            status: status.as_u16(),
            body,
        });
    }

    #[derive(Deserialize)]
    struct TokenResp {
        #[serde(default)]
        token: String,
        #[serde(default)]
        access_token: String,
    }
    let body: TokenResp = resp.json().map_err(|source| TransportError::Http {
        url: challenge.realm.clone(),
        source,
    })?;
    let tok = if !body.token.is_empty() {
        body.token
    } else {
        body.access_token
    };
    if tok.is_empty() {
        return Err(TransportError::AuthRequired {
            registry: challenge
                .service
                .clone()
                .unwrap_or_else(|| challenge.realm.clone()),
        });
    }
    Ok(tok)
}

/// Apply the current cached bearer token (or a raw PAT if that's
/// all we have). Basic creds don't get attached directly — they're
/// forwarded to the realm endpoint via `fetch_token` on a 401.
pub(crate) fn apply_bearer(
    req: reqwest::blocking::RequestBuilder,
    token_cache: &TokenCache,
    creds: Option<&Credentials>,
) -> reqwest::blocking::RequestBuilder {
    if let Some(tok) = &token_cache.token {
        return req.bearer_auth(tok);
    }
    if let Some(Credentials::Bearer { token }) = creds {
        return req.bearer_auth(token);
    }
    req
}

/// GET with the retry-on-401-bearer-challenge pattern. Shared
/// between `oci_fetcher` and `oci_puller` — both need the same
/// auth + decorate shape for pulls. The `decorate` closure lets
/// callers add `Accept:` headers per request type.
/// Backwards-compatible wrapper that bounds the response at
/// [`MAX_BLOB_BYTES`] (the larger of the two ceilings — safe as a
/// catch-all for callers that don't distinguish manifest from blob).
/// Callers that know they're pulling a manifest should use
/// [`get_with_auth_capped`] with [`MAX_MANIFEST_BYTES`].
pub(crate) fn get_with_auth(
    client: &reqwest::blocking::Client,
    url: &str,
    registry: &str,
    creds: Option<&Credentials>,
    token_cache: &mut TokenCache,
    decorate: impl Fn(reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder,
) -> Result<Vec<u8>, TransportError> {
    get_with_auth_capped(
        client,
        url,
        registry,
        creds,
        token_cache,
        MAX_BLOB_BYTES,
        decorate,
    )
}

/// GET with the retry-on-401 dance, bounding the response body at
/// `max_bytes`.
pub(crate) fn get_with_auth_capped(
    client: &reqwest::blocking::Client,
    url: &str,
    registry: &str,
    creds: Option<&Credentials>,
    token_cache: &mut TokenCache,
    max_bytes: u64,
    decorate: impl Fn(reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder,
) -> Result<Vec<u8>, TransportError> {
    let req = apply_bearer(decorate(client.get(url)), token_cache, creds);
    let resp = req.send().map_err(|source| TransportError::Http {
        url: url.to_string(),
        source,
    })?;
    if resp.status().as_u16() != 401 {
        return ensure_ok(resp, url, max_bytes);
    }

    let challenge =
        BearerChallenge::from_resp(&resp).ok_or_else(|| TransportError::AuthRequired {
            registry: registry.to_string(),
        })?;
    let token = fetch_token(client, &challenge, creds)?;
    token_cache.token = Some(token.clone());

    let retry_req = decorate(client.get(url)).bearer_auth(&token);
    let retry = retry_req.send().map_err(|source| TransportError::Http {
        url: url.to_string(),
        source,
    })?;
    if retry.status().as_u16() == 401 {
        return Err(TransportError::AuthRequired {
            registry: registry.to_string(),
        });
    }
    ensure_ok(retry, url, max_bytes)
}

/// Truncate `body` to at most [`MAX_ERROR_BODY_CHARS`] characters on a
/// UTF-8 char boundary. Slicing `&body[..300]` panics when byte 300
/// lands inside a multibyte sequence — a malicious registry can craft
/// an error body that triggers exactly that. Iterating by `char`
/// never splits a code point.
fn truncate_error_body(body: &str) -> String {
    body.chars().take(MAX_ERROR_BODY_CHARS).collect()
}

/// Read a response body into memory, aborting if it exceeds
/// `max_bytes`. Rejects early when `Content-Length` already declares
/// an oversize body; otherwise streams through a capped reader so a
/// registry that lies about (or omits) the length can't push us past
/// the ceiling. The +1 read target lets a body sitting exactly at the
/// cap through while still catching the first byte over it.
fn read_body_capped(
    resp: reqwest::blocking::Response,
    url: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, TransportError> {
    if let Some(declared) = resp.content_length() {
        if declared > max_bytes {
            return Err(TransportError::ResponseTooLarge {
                url: url.to_string(),
                limit: max_bytes,
                declared: Some(declared),
            });
        }
    }
    let mut buf = Vec::new();
    let mut reader = resp.take(max_bytes.saturating_add(1));
    reader
        .read_to_end(&mut buf)
        .map_err(|source| TransportError::BodyRead {
            url: url.to_string(),
            source,
        })?;
    if buf.len() as u64 > max_bytes {
        return Err(TransportError::ResponseTooLarge {
            url: url.to_string(),
            limit: max_bytes,
            declared: None,
        });
    }
    Ok(buf)
}

/// Success-path unwrap: on 2xx pull the body as bytes (bounded by
/// `max_bytes`), on anything else capture a short body for
/// diagnostics.
pub(crate) fn ensure_ok(
    resp: reqwest::blocking::Response,
    url: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, TransportError> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(TransportError::Status {
            url: url.to_string(),
            status: status.as_u16(),
            body: truncate_error_body(&body),
        });
    }
    read_body_capped(resp, url, max_bytes)
}

// ---------------------------------------------------------------------------
// Tests — parser coverage + challenge parser. HTTP-requiring tests
// live in the integration-test crates.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_refs() {
        assert_eq!(
            parse_ref("oci://ghcr.io/acme/app").unwrap(),
            OciRef {
                registry: "ghcr.io".into(),
                repository: "acme/app".into()
            }
        );
        assert_eq!(
            parse_ref("oci://registry-1.docker.io/bitnamicharts/nginx").unwrap(),
            OciRef {
                registry: "registry-1.docker.io".into(),
                repository: "bitnamicharts/nginx".into()
            }
        );
    }

    #[test]
    fn rejects_refs_without_scheme_or_repo() {
        assert!(matches!(
            parse_ref("ghcr.io/x/y"),
            Err(TransportError::BadRef(_))
        ));
        assert!(matches!(
            parse_ref("oci://ghcr.io"),
            Err(TransportError::BadRef(_))
        ));
        assert!(matches!(parse_ref(""), Err(TransportError::BadRef(_))));
    }

    #[test]
    fn truncate_error_body_is_char_boundary_safe() {
        // A body whose multibyte char straddles byte 300 used to panic
        // on `&body[..300]`. `é` is 2 bytes in UTF-8, so 299 of them is
        // 598 bytes with a char boundary landing inside the window.
        let body = "é".repeat(400);
        assert!(body.len() > 300, "fixture must exceed the byte cap");
        let truncated = truncate_error_body(&body);
        // Char-count cap, not byte cap.
        assert_eq!(truncated.chars().count(), 300);
        // Must be valid UTF-8 (it is, by construction) and not panic.
        assert_eq!(truncated, "é".repeat(300));
    }

    #[test]
    fn truncate_error_body_passes_short_bodies_through() {
        assert_eq!(truncate_error_body("short"), "short");
        assert_eq!(truncate_error_body(""), "");
    }

    #[test]
    fn truncate_error_body_handles_multibyte_at_exact_boundary() {
        // A 4-byte emoji repeated so byte index 300 lands mid-sequence —
        // the old byte slice would panic here.
        let body = "🦀".repeat(200);
        let truncated = truncate_error_body(&body);
        assert_eq!(truncated.chars().count(), 200);
        assert_eq!(truncated, "🦀".repeat(200));
    }

    #[test]
    fn registry_scheme_uses_https_for_real_registries() {
        assert_eq!(registry_scheme("ghcr.io"), "https");
        assert_eq!(registry_scheme("registry-1.docker.io"), "https");
        assert_eq!(registry_scheme("registry.example.com:5000"), "https");
    }

    #[test]
    fn registry_scheme_uses_http_for_loopback() {
        // Bare loopback hostnames + with port — both must downgrade to
        // http so local mock registries (incl. our test fixtures) work
        // without TLS termination.
        assert_eq!(registry_scheme("localhost"), "http");
        assert_eq!(registry_scheme("localhost:5000"), "http");
        assert_eq!(registry_scheme("127.0.0.1"), "http");
        assert_eq!(registry_scheme("127.0.0.1:8443"), "http");
        assert_eq!(registry_scheme("::1"), "http");
        assert_eq!(registry_scheme("[::1]"), "http");
    }
}
