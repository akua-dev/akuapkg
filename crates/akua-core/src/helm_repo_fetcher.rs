//! Resolve Helm charts from classic HTTPS Helm repositories
//! (`index.yaml` + versioned `.tgz`). Content-pinned like git deps:
//! the resolved tarball's tree sha256 is recorded in `akua.lock` and
//! verified on every pull. No cosign; `.prov`/GPG is a future opt-in.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::Read;

/// Ceiling on a repo `index.yaml` body. Indexes list every published
/// chart version; even a large public repo is a few MiB. 32 MiB bounds
/// a hostile server streaming an endless index into memory.
const MAX_INDEX_BYTES: u64 = 32 * 1024 * 1024;

/// Ceiling on a downloaded chart `.tgz`. Helm charts are sub-MiB in
/// practice; 1 GiB bounds the buffered download without OOM. The digest
/// is only verifiable after the full body is read, so this cap is the
/// backstop against a server that streams without bound.
const MAX_TARBALL_BYTES: u64 = 1024 * 1024 * 1024;

/// Ceiling on the *decompressed* chart tree. The pinned digest covers
/// the compressed `.tgz` only, so a small valid-digest gzip could
/// expand to fill the disk. 2 GiB is generous for any real chart.
const MAX_DECOMPRESSED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Sentinel embedded in the `io::Error` raised by the decompression
/// cap so `extract_and_hash` can map it to a typed error.
const DECOMPRESSION_LIMIT_MARKER: &str = "akua: decompressed size limit exceeded";

/// A `Read` adapter that aborts once more than `limit` bytes have been
/// pulled through it — used to bound the decompressed chart tree.
struct CappedReader<R> {
    inner: R,
    read: u64,
    limit: u64,
}

impl<R: Read> Read for CappedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read = self.read.saturating_add(n as u64);
        if self.read > self.limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                DECOMPRESSION_LIMIT_MARKER,
            ));
        }
        Ok(n)
    }
}

/// Walk an error's `source()` chain looking for `needle` in any
/// `Display` rendering. `tar` wraps the underlying io error, so the
/// decompression-cap sentinel can sit a level or two down.
fn source_chain_contains(e: &(dyn std::error::Error + 'static), needle: &str) -> bool {
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(err) = cur {
        if err.to_string().contains(needle) {
            return true;
        }
        cur = err.source();
    }
    false
}

/// One entry in a repo's `index.yaml` `entries.<chart>[]` list. Only
/// the fields akua needs; serde ignores the rest.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct IndexEntry {
    pub version: String,
    #[serde(default)]
    pub urls: Vec<String>,
}

/// A parsed `index.yaml`: chart name → its published entries.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RepoIndex {
    #[serde(default)]
    pub entries: BTreeMap<String, Vec<IndexEntry>>,
}

/// Parse an `index.yaml` byte slice.
pub fn parse_index(bytes: &[u8]) -> Result<RepoIndex, HelmRepoFetchError> {
    serde_yaml::from_slice(bytes).map_err(|e| HelmRepoFetchError::IndexParse {
        detail: e.to_string(),
    })
}

#[derive(Debug, thiserror::Error)]
pub enum HelmRepoFetchError {
    #[error("parsing index.yaml: {detail}")]
    IndexParse { detail: String },
    #[error("chart `{chart}` not found in repo index")]
    ChartNotFound { chart: String },
    #[error("no version of `{chart}` satisfies `{req}`; available: {available}")]
    NoMatchingVersion {
        chart: String,
        req: String,
        available: String,
    },
    #[error("chart `{chart}`@`{version}` has no download URL in the index")]
    NoUrl { chart: String, version: String },
    #[error("invalid version requirement `{req}`: {detail}")]
    BadRequirement { req: String, detail: String },
    #[error("fetching {url}: {detail}")]
    Http { url: String, detail: String },
    #[error(
        "index for `{chart}` points its tarball at an http:// URL (`{url}`) but the repo was \
         fetched over https:// — refusing the scheme downgrade"
    )]
    SchemeDowngrade { chart: String, url: String },
    #[error("response from {url} exceeds the {limit}-byte ceiling")]
    ResponseTooLarge { url: String, limit: u64 },
    #[error(
        "decompressed chart `{chart}` exceeds the {limit}-byte ceiling (possible decompression bomb)"
    )]
    DecompressionLimit { chart: String, limit: u64 },
    #[error("digest mismatch for `{chart}`: expected {expected}, got {actual}")]
    DigestMismatch {
        chart: String,
        expected: String,
        actual: String,
    },
    #[error("offline and `{chart}` is not in the cache — run `akua add` online first")]
    OfflineCacheMiss { chart: String },
    #[error("io error at {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Select the `.tgz` URL for `chart` satisfying `version_req`.
///
/// `version_req` is parsed as a semver requirement: an exact version
/// like `0.62.0` matches only itself; a range like `>=0.60, <0.63`
/// selects the **highest** published version that satisfies it.
/// Pre-releases are excluded unless the requirement names one.
/// Returns `(resolved_version, tarball_url)`.
pub fn select_version(
    index: &RepoIndex,
    chart: &str,
    version_req: &str,
) -> Result<(String, String), HelmRepoFetchError> {
    let entries = index
        .entries
        .get(chart)
        .ok_or_else(|| HelmRepoFetchError::ChartNotFound {
            chart: chart.to_string(),
        })?;

    let req = parse_version_req(version_req)?;

    let mut best: Option<(semver::Version, &IndexEntry)> = None;
    for entry in entries {
        // Helm-compatibility: a leading `v` is a git-tag convention, not
        // part of the SemVer spec, but Helm's resolver (Masterminds/semver)
        // tolerates it by stripping. Strip a single leading `v` before
        // parsing; skip only entries that are STILL unparseable afterwards
        // rather than aborting the whole resolution on the first bad one
        // (the loft-sh `vcluster` index ships 7 `v`-prefixed pre-release
        // tags alongside 625 plain-SemVer versions). Comparisons use the
        // normalized `Version`; the download URL is taken from the entry's
        // own `urls`, so normalization never perturbs URL/digest selection.
        let raw = entry.version.strip_prefix('v').unwrap_or(&entry.version);
        let ver = match semver::Version::parse(raw) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if !req.matches(&ver) {
            continue;
        }
        if best.as_ref().map(|(b, _)| &ver > b).unwrap_or(true) {
            best = Some((ver, entry));
        }
    }

    let (ver, entry) = best.ok_or_else(|| {
        let mut available: Vec<&str> = entries.iter().map(|e| e.version.as_str()).collect();
        available.sort_unstable();
        HelmRepoFetchError::NoMatchingVersion {
            chart: chart.to_string(),
            req: version_req.to_string(),
            available: available.join(", "),
        }
    })?;

    let url = entry
        .urls
        .first()
        .ok_or_else(|| HelmRepoFetchError::NoUrl {
            chart: chart.to_string(),
            version: ver.to_string(),
        })?
        .clone();
    Ok((ver.to_string(), url))
}

fn parse_version_req(version_req: &str) -> Result<semver::VersionReq, HelmRepoFetchError> {
    let trimmed = version_req.trim();
    let normalized = trimmed.strip_prefix('v').unwrap_or(trimmed);
    let req = if semver::Version::parse(normalized).is_ok() {
        Cow::Owned(format!("={normalized}"))
    } else {
        Cow::Borrowed(trimmed)
    };
    semver::VersionReq::parse(&req).map_err(|e| HelmRepoFetchError::BadRequirement {
        req: version_req.to_string(),
        detail: e.to_string(),
    })
}

/// Resolve a tarball URL from `index.yaml` against the repo base.
/// Absolute (`http://`/`https://`) URLs pass through; relative ones
/// are joined to `<repo>/`.
///
/// Scheme-downgrade guard: if the repo itself was fetched over
/// `https://` (the index came from a TLS-protected channel) but the
/// index hands back an absolute `http://` tarball URL, refuse it. A
/// MITM that can rewrite an index entry could otherwise downgrade the
/// chart download to plaintext; the digest pin only protects against
/// tampering *after* a trusted digest is recorded, not the first pull.
/// `chart` is threaded through only for the error message.
pub fn resolve_tarball_url(
    repo: &str,
    url: &str,
    chart: &str,
) -> Result<String, HelmRepoFetchError> {
    if url.starts_with("https://") {
        return Ok(url.to_string());
    }
    if url.starts_with("http://") {
        if repo.starts_with("https://") {
            return Err(HelmRepoFetchError::SchemeDowngrade {
                chart: chart.to_string(),
                url: url.to_string(),
            });
        }
        return Ok(url.to_string());
    }
    Ok(format!(
        "{}/{}",
        repo.trim_end_matches('/'),
        url.trim_start_matches('/')
    ))
}

use std::path::{Path, PathBuf};

/// Result of a successful helm-repo fetch.
#[derive(Debug, Clone)]
pub struct Fetched {
    /// Absolute path to the unpacked chart root (contains `Chart.yaml`).
    pub root_dir: PathBuf,
    /// `sha256:<hex>` of the pulled `.tgz` bytes.
    pub digest: String,
    /// Resolved exact version (set by `fetch`, echoed for the lockfile).
    pub version: String,
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

fn cache_dir_for(cache_root: &Path, digest: &str) -> PathBuf {
    cache_root.join(digest.trim_start_matches("sha256:"))
}

/// Unpack `tgz` into the content-addressed cache and return the root.
/// `chart_dir_name` is the top-level directory helm packs the chart under
/// (the chart's own name), used to locate the unpacked root.
pub fn extract_and_hash(
    tgz: &[u8],
    cache_root: &Path,
    chart_dir_name: &str,
) -> Result<Fetched, HelmRepoFetchError> {
    let digest = format!("sha256:{}", sha256_hex(tgz));
    let dest = cache_dir_for(cache_root, &digest);
    let root = dest.join(chart_dir_name);
    if !root.join("Chart.yaml").is_file() {
        unpack_tgz_capped(tgz, &dest, chart_dir_name, MAX_DECOMPRESSED_BYTES)?;
    }
    Ok(Fetched {
        root_dir: root,
        digest,
        version: String::new(),
    })
}

/// Decompress + unpack `tgz` into `dest`, capping the *decompressed*
/// byte stream at `decompressed_limit`. The pinned digest covers the
/// compressed `.tgz` only, so a small valid-digest gzip could expand to
/// fill the disk; the cap surfaces as a typed `DecompressionLimit`.
/// Split from `extract_and_hash` so tests can trip the guard with a
/// small ceiling without materializing a multi-GiB fixture.
fn unpack_tgz_capped(
    tgz: &[u8],
    dest: &Path,
    chart_dir_name: &str,
    decompressed_limit: u64,
) -> Result<(), HelmRepoFetchError> {
    std::fs::create_dir_all(dest).map_err(|source| HelmRepoFetchError::Io {
        path: dest.to_path_buf(),
        source,
    })?;
    let gz = flate2::read::GzDecoder::new(tgz);
    let mut ar = tar::Archive::new(CappedReader {
        inner: gz,
        read: 0,
        limit: decompressed_limit,
    });
    // tar crate rejects `..`/absolute members by default (no
    // set_overwrite/preserve escape), so extraction stays within `dest`.
    ar.unpack(dest).map_err(|source| {
        if source_chain_contains(&source, DECOMPRESSION_LIMIT_MARKER) {
            HelmRepoFetchError::DecompressionLimit {
                chart: chart_dir_name.to_string(),
                limit: decompressed_limit,
            }
        } else {
            HelmRepoFetchError::Io {
                path: dest.to_path_buf(),
                source,
            }
        }
    })
}

/// Offline path: return the cached unpack for a pinned digest, if present.
/// `chart_dir_name` is the chart's own name — the top-level directory the
/// online path unpacked into. Locate deterministically rather than scanning.
pub fn fetch_from_cache(cache_root: &Path, digest: &str, chart_dir_name: &str) -> Option<Fetched> {
    let dest = cache_dir_for(cache_root, digest);
    // Deterministic: the online path unpacks to `<dest>/<chart_dir_name>/`.
    let named = dest.join(chart_dir_name);
    let root = if named.join("Chart.yaml").is_file() {
        named
    } else if dest.join("Chart.yaml").is_file() {
        // Chart packed at the archive root rather than under a dir.
        dest.clone()
    } else {
        return None;
    };
    Some(Fetched {
        root_dir: root,
        digest: digest.to_string(),
        version: String::new(),
    })
}

/// Options for an online fetch.
pub struct FetchOpts<'a> {
    /// Lockfile-pinned digest; when set, the pulled tarball's sha256
    /// must match or the fetch fails.
    pub expected_digest: Option<&'a str>,
    /// Host-keyed basic-auth map for private repos.
    pub auth: Option<&'a crate::host_auth::HostAuthMap>,
}

/// Fetch `chart`@`version_req` from `repo` over HTTPS: GET index.yaml,
/// select the version, download the `.tgz`, verify/assign the digest,
/// unpack into the cache. Returns the unpacked root + resolved version.
pub fn fetch(
    repo: &str,
    chart: &str,
    version_req: &str,
    cache_root: &Path,
    opts: &FetchOpts<'_>,
) -> Result<Fetched, HelmRepoFetchError> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("akua/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| HelmRepoFetchError::Http {
            url: repo.to_string(),
            detail: e.to_string(),
        })?;

    let index_url = format!("{}/index.yaml", repo.trim_end_matches('/'));
    let index_bytes = http_get(&client, &index_url, opts.auth, MAX_INDEX_BYTES)?;
    let index = parse_index(&index_bytes)?;
    let (version, tgz_url) = select_version(&index, chart, version_req)?;
    let abs_url = resolve_tarball_url(repo, &tgz_url, chart)?;
    let tgz = http_get(&client, &abs_url, opts.auth, MAX_TARBALL_BYTES)?;

    let digest = format!("sha256:{}", sha256_hex(&tgz));
    if let Some(expected) = opts.expected_digest {
        if expected != digest {
            return Err(HelmRepoFetchError::DigestMismatch {
                chart: chart.to_string(),
                expected: expected.to_string(),
                actual: digest,
            });
        }
    }
    let mut fetched = extract_and_hash(&tgz, cache_root, chart)?;
    fetched.version = version;
    Ok(fetched)
}

fn http_get(
    client: &reqwest::blocking::Client,
    url: &str,
    auth: Option<&crate::host_auth::HostAuthMap>,
    max_bytes: u64,
) -> Result<Vec<u8>, HelmRepoFetchError> {
    let mut req = client.get(url);
    if let Some(map) = auth {
        if let Some(creds) = crate::host_auth::lookup(map, url) {
            req = req.basic_auth(&creds.username, Some(&creds.password));
        }
    }
    let resp = req.send().map_err(|e| HelmRepoFetchError::Http {
        url: url.to_string(),
        detail: e.to_string(),
    })?;
    if !resp.status().is_success() {
        return Err(HelmRepoFetchError::Http {
            url: url.to_string(),
            detail: format!("HTTP {}", resp.status()),
        });
    }
    // Reject early when the server declares an oversize body, then
    // stream through a capped reader so a missing/​lying Content-Length
    // can't push us past the ceiling.
    if let Some(declared) = resp.content_length() {
        if declared > max_bytes {
            return Err(HelmRepoFetchError::ResponseTooLarge {
                url: url.to_string(),
                limit: max_bytes,
            });
        }
    }
    let mut buf = Vec::new();
    resp.take(max_bytes.saturating_add(1))
        .read_to_end(&mut buf)
        .map_err(|e| HelmRepoFetchError::Http {
            url: url.to_string(),
            detail: e.to_string(),
        })?;
    if buf.len() as u64 > max_bytes {
        return Err(HelmRepoFetchError::ResponseTooLarge {
            url: url.to_string(),
            limit: max_bytes,
        });
    }
    Ok(buf)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Build a minimal chart `.tgz` in memory: `demo/Chart.yaml`.
    /// Shared with `chart_resolver` tests, which prime a cache with it.
    #[cfg(test)]
    pub(crate) fn demo_tgz() -> Vec<u8> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        {
            let mut tar = tar::Builder::new(&mut gz);
            let chart_yaml = b"apiVersion: v2\nname: demo\nversion: 0.1.0\n";
            let mut h = tar::Header::new_gnu();
            h.set_size(chart_yaml.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            tar.append_data(&mut h, "demo/Chart.yaml", &chart_yaml[..])
                .unwrap();
            tar.finish().unwrap();
        }
        gz.finish().unwrap()
    }

    const FIXTURE: &[u8] = br#"
apiVersion: v1
entries:
  temporal:
    - version: 0.62.0
      urls: ["https://go.temporal.io/helm-charts/temporal-0.62.0.tgz"]
    - version: 0.61.0
      urls: ["temporal-0.61.0.tgz"]
"#;

    #[test]
    fn parses_index_entries() {
        let idx = parse_index(FIXTURE).expect("parse");
        let entries = idx.entries.get("temporal").expect("temporal present");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].version, "0.62.0");
        assert_eq!(entries[1].urls[0], "temporal-0.61.0.tgz");
    }

    #[test]
    fn selects_exact_version() {
        let idx = parse_index(FIXTURE).unwrap();
        let (v, url) = select_version(&idx, "temporal", "0.61.0").unwrap();
        assert_eq!(v, "0.61.0");
        assert_eq!(url, "temporal-0.61.0.tgz");
    }

    #[test]
    fn selects_highest_in_range() {
        let idx = parse_index(FIXTURE).unwrap();
        let (v, _) = select_version(&idx, "temporal", ">=0.60, <0.63").unwrap();
        assert_eq!(v, "0.62.0", "highest satisfying version");
    }

    /// A leading `v` is a git-tag convention, not part of the SemVer
    /// spec. Helm's own resolver (Masterminds/semver) strips it; akua
    /// must too. The entry `v0.28.0-next.12` is a valid SemVer once the
    /// `v` is removed — it must be ACCEPTED (and normalized), not skipped.
    #[test]
    fn strips_leading_v_and_accepts_prefixed_prerelease() {
        const IDX: &[u8] = br#"
apiVersion: v1
entries:
  vcluster:
    - version: v0.28.0-next.12
      urls: ["vcluster-v0.28.0-next.12.tgz"]
"#;
        let idx = parse_index(IDX).unwrap();
        // A pre-release req only matches the matching pre-release; this
        // proves the `v`-prefixed entry was parsed (not skipped).
        let (v, url) = select_version(&idx, "vcluster", "=0.28.0-next.12").unwrap();
        assert_eq!(
            v, "0.28.0-next.12",
            "leading `v` stripped, version normalized"
        );
        assert_eq!(
            url, "vcluster-v0.28.0-next.12.tgz",
            "download URL comes from the original entry, untouched by normalization"
        );
    }

    /// A genuinely-malformed entry must be skipped (not abort the whole
    /// resolution), letting a valid sibling resolve.
    #[test]
    fn skips_truly_malformed_entries() {
        const IDX: &[u8] = br#"
apiVersion: v1
entries:
  vcluster:
    - version: garbage
      urls: ["garbage.tgz"]
    - version: 0.34.0
      urls: ["vcluster-0.34.0.tgz"]
"#;
        let idx = parse_index(IDX).unwrap();
        let (v, url) = select_version(&idx, "vcluster", "0.34.0").unwrap();
        assert_eq!(v, "0.34.0");
        assert_eq!(url, "vcluster-0.34.0.tgz");
    }

    /// Mix of `v`-prefixed pre-releases, plain releases, and garbage —
    /// requesting `0.34.0` must resolve to exactly `0.34.0` rather than
    /// aborting on the first non-strict entry.
    #[test]
    fn resolves_target_amid_v_prefixed_and_garbage() {
        const IDX: &[u8] = br#"
apiVersion: v1
entries:
  vcluster:
    - version: v0.28.0-next.12
      urls: ["vcluster-v0.28.0-next.12.tgz"]
    - version: v0.28.0-next.5
      urls: ["vcluster-v0.28.0-next.5.tgz"]
    - version: garbage
      urls: ["garbage.tgz"]
    - version: 0.34.0
      urls: ["vcluster-0.34.0.tgz"]
"#;
        let idx = parse_index(IDX).unwrap();
        let (v, url) = select_version(&idx, "vcluster", "0.34.0").unwrap();
        assert_eq!(v, "0.34.0");
        assert_eq!(url, "vcluster-0.34.0.tgz");
    }

    /// Regression mirroring the live loft-sh `vcluster` index that broke
    /// `akua lock`: plain releases (0.34.0, 0.34.1), a few `v`-prefixed
    /// pre-release tags (the 7 that aborted strict parsing), and a couple
    /// plain pre-releases. Requesting `0.34.0` must resolve to exactly
    /// `0.34.0` and excludes pre-releases (req names none).
    #[test]
    fn resolves_loftsh_like_vcluster_index() {
        const IDX: &[u8] = br#"
apiVersion: v1
entries:
  vcluster:
    - version: v0.28.0-next.12
      urls: ["charts/vcluster-v0.28.0-next.12.tgz"]
    - version: v0.28.0-next.11
      urls: ["charts/vcluster-v0.28.0-next.11.tgz"]
    - version: 0.34.1
      urls: ["charts/vcluster-0.34.1.tgz"]
    - version: 0.34.0
      urls: ["charts/vcluster-0.34.0.tgz"]
    - version: 0.34.0-rc.1
      urls: ["charts/vcluster-0.34.0-rc.1.tgz"]
    - version: 0.33.0
      urls: ["charts/vcluster-0.33.0.tgz"]
"#;
        let idx = parse_index(IDX).unwrap();
        // A bare version pins exactly. The point of the regression is
        // that the `v`-prefixed pre-release siblings no longer abort the
        // whole resolution before the target is even considered.
        let (v, url) = select_version(&idx, "vcluster", "0.34.0").unwrap();
        assert_eq!(
            v, "0.34.0",
            "exact target resolves despite v-prefixed siblings"
        );
        assert_eq!(url, "charts/vcluster-0.34.0.tgz");

        let (prefixed, _) = select_version(&idx, "vcluster", "v0.34.0").unwrap();
        assert_eq!(prefixed, "0.34.0", "user-supplied `v` is tolerated");

        // And an explicit caret request resolves to the highest plain
        // release in the 0.34.x line (0.34.1), confirming pre-releases
        // stay excluded.
        let (caret, _) = select_version(&idx, "vcluster", "^0.34.0").unwrap();
        assert_eq!(
            caret, "0.34.1",
            "caret picks highest non-prerelease in range"
        );
    }

    #[test]
    fn errors_when_no_version_satisfies() {
        let idx = parse_index(FIXTURE).unwrap();
        let err = select_version(&idx, "temporal", ">=1.0.0").unwrap_err();
        assert!(matches!(err, HelmRepoFetchError::NoMatchingVersion { .. }));
    }

    #[test]
    fn errors_when_chart_absent() {
        let idx = parse_index(FIXTURE).unwrap();
        let err = select_version(&idx, "missing", "1.0.0").unwrap_err();
        assert!(matches!(err, HelmRepoFetchError::ChartNotFound { .. }));
    }

    #[test]
    fn resolves_relative_and_absolute_urls() {
        let repo = "https://go.temporal.io/helm-charts";
        assert_eq!(
            resolve_tarball_url(
                repo,
                "https://cdn.example.com/temporal-0.62.0.tgz",
                "temporal"
            )
            .unwrap(),
            "https://cdn.example.com/temporal-0.62.0.tgz"
        );
        assert_eq!(
            resolve_tarball_url(repo, "temporal-0.61.0.tgz", "temporal").unwrap(),
            "https://go.temporal.io/helm-charts/temporal-0.61.0.tgz"
        );
        assert_eq!(
            resolve_tarball_url(
                "https://go.temporal.io/helm-charts/",
                "temporal-0.61.0.tgz",
                "temporal"
            )
            .unwrap(),
            "https://go.temporal.io/helm-charts/temporal-0.61.0.tgz"
        );
    }

    #[test]
    fn rejects_http_tarball_url_from_https_repo() {
        // A MITM that rewrites an index entry could point the chart
        // download at plaintext http; refuse the downgrade.
        let repo = "https://go.temporal.io/helm-charts";
        let err = resolve_tarball_url(repo, "http://evil.example.com/temporal.tgz", "temporal")
            .unwrap_err();
        assert!(matches!(err, HelmRepoFetchError::SchemeDowngrade { .. }));
    }

    #[test]
    fn allows_http_tarball_url_from_http_repo() {
        // A repo intentionally served over plaintext http (local mirror,
        // test server) may hand back http tarball URLs — no downgrade.
        let repo = "http://127.0.0.1:8080/charts";
        assert_eq!(
            resolve_tarball_url(repo, "http://127.0.0.1:8080/demo.tgz", "demo").unwrap(),
            "http://127.0.0.1:8080/demo.tgz"
        );
    }

    #[test]
    fn extract_and_hash_unpacks_chart_and_pins_digest() {
        let tgz = demo_tgz();

        let cache = tempfile::tempdir().unwrap();
        let first = extract_and_hash(&tgz, cache.path(), "demo").expect("extract");
        assert!(first.root_dir.join("Chart.yaml").is_file());
        assert!(first.digest.starts_with("sha256:"));

        // Deterministic: same bytes → same digest, and it lands in the cache.
        let cached = fetch_from_cache(cache.path(), &first.digest, "demo").expect("cached");
        assert_eq!(cached.digest, first.digest);
        assert!(cached.root_dir.join("Chart.yaml").is_file());
    }

    #[test]
    fn unpack_tgz_capped_rejects_decompression_bomb() {
        // ~1 MiB of zeros compresses to a tiny tgz but blows past the
        // 4 KiB test cap — the guard must abort with a typed error
        // rather than writing the full payload to disk.
        let mut buf = Vec::new();
        {
            let gz = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
            let mut tar = tar::Builder::new(gz);
            let big = vec![0u8; 1024 * 1024];
            let mut h = tar::Header::new_gnu();
            h.set_size(big.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            tar.append_data(&mut h, "demo/Chart.yaml", &big[..])
                .unwrap();
            tar.into_inner().unwrap().finish().unwrap();
        }
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("unpacked");
        let err = unpack_tgz_capped(&buf, &dest, "demo", 4096).unwrap_err();
        assert!(
            matches!(err, HelmRepoFetchError::DecompressionLimit { .. }),
            "expected DecompressionLimit, got {err:?}"
        );
    }

    /// A minimal in-process HTTP/1.1 server for the online-fetch tests.
    /// Binds an ephemeral port, then serves exactly `requests` connections
    /// from a spawned thread. Two routes: `GET /index.yaml` and the chart
    /// `.tgz`; everything else is 404. No external network, no new deps —
    /// just `std::net::TcpListener`. The handler tolerates client-side
    /// aborts (a digest-pin failure never downloads the tarball) so the
    /// thread exits cleanly whether or not every route is hit.
    fn serve_demo_repo(requests: usize) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let port = listener.local_addr().expect("local addr").port();

        std::thread::spawn(move || {
            for _ in 0..requests {
                let (mut stream, _) = match listener.accept() {
                    Ok(c) => c,
                    Err(_) => return,
                };
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .ok();

                // Read just the request line; the body is irrelevant for GET.
                let mut buf = [0u8; 1024];
                let n = stream.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]);
                let path = head
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("");

                let body: Vec<u8> = match path {
                    "/index.yaml" => format!(
                        // Two versions; 0.2.0 uses a RELATIVE url to exercise
                        // resolve_tarball_url, 0.1.0 an absolute one.
                        "apiVersion: v1\n\
                         entries:\n\
                         \x20\x20demo:\n\
                         \x20\x20\x20\x20- version: 0.2.0\n\
                         \x20\x20\x20\x20\x20\x20urls: [\"demo-0.2.0.tgz\"]\n\
                         \x20\x20\x20\x20- version: 0.1.0\n\
                         \x20\x20\x20\x20\x20\x20urls: [\"http://127.0.0.1:{port}/demo-0.1.0.tgz\"]\n"
                    )
                    .into_bytes(),
                    "/demo-0.2.0.tgz" => demo_tgz(),
                    _ => {
                        let _ = stream
                            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                        continue;
                    }
                };

                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                // A client abort mid-write is fine — ignore the error.
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });

        format!("http://127.0.0.1:{port}")
    }

    #[test]
    fn fetch_online_selects_highest_and_pins_tgz_digest() {
        let repo = serve_demo_repo(2);
        let cache = tempfile::tempdir().unwrap();

        let fetched = fetch(
            &repo,
            "demo",
            ">=0.1, <0.3",
            cache.path(),
            &FetchOpts {
                expected_digest: None,
                auth: None,
            },
        )
        .expect("online fetch");

        assert_eq!(fetched.version, "0.2.0", "highest version in range");
        assert!(fetched.root_dir.join("Chart.yaml").is_file());
        // Digest pins the pulled `.tgz` bytes, not the unpacked tree.
        assert_eq!(
            fetched.digest,
            format!("sha256:{}", sha256_hex(&demo_tgz()))
        );
    }

    #[test]
    fn fetch_online_rejects_digest_mismatch() {
        let repo = serve_demo_repo(2);
        let cache = tempfile::tempdir().unwrap();

        let wrong = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
        let err = fetch(
            &repo,
            "demo",
            ">=0.1, <0.3",
            cache.path(),
            &FetchOpts {
                expected_digest: Some(wrong),
                auth: None,
            },
        )
        .expect_err("digest mismatch must fail the fetch");

        assert!(matches!(err, HelmRepoFetchError::DigestMismatch { .. }));
        // Guard fires before extraction: the cache stays empty.
        assert!(
            std::fs::read_dir(cache.path()).unwrap().next().is_none(),
            "no chart should be unpacked on a pinned-digest mismatch"
        );
    }
}
