//! Resolve Helm charts from classic HTTPS Helm repositories
//! (`index.yaml` + versioned `.tgz`). Content-pinned like git deps:
//! the resolved tarball's tree sha256 is recorded in `akua.lock` and
//! verified on every pull. No cosign; `.prov`/GPG is a future opt-in.

use std::collections::BTreeMap;

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
    #[error("invalid version `{version}` in index for `{chart}`: {detail}")]
    BadVersion {
        chart: String,
        version: String,
        detail: String,
    },
    #[error("invalid version requirement `{req}`: {detail}")]
    BadRequirement { req: String, detail: String },
    #[error("fetching {url}: {detail}")]
    Http { url: String, detail: String },
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

    let req =
        semver::VersionReq::parse(version_req).map_err(|e| HelmRepoFetchError::BadRequirement {
            req: version_req.to_string(),
            detail: e.to_string(),
        })?;

    let mut best: Option<(semver::Version, &IndexEntry)> = None;
    for entry in entries {
        let ver =
            semver::Version::parse(&entry.version).map_err(|e| HelmRepoFetchError::BadVersion {
                chart: chart.to_string(),
                version: entry.version.clone(),
                detail: e.to_string(),
            })?;
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

/// Resolve a tarball URL from `index.yaml` against the repo base.
/// Absolute (`http://`/`https://`) URLs pass through; relative ones
/// are joined to `<repo>/`.
pub fn resolve_tarball_url(repo: &str, url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_string();
    }
    format!(
        "{}/{}",
        repo.trim_end_matches('/'),
        url.trim_start_matches('/')
    )
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
        std::fs::create_dir_all(&dest).map_err(|source| HelmRepoFetchError::Io {
            path: dest.clone(),
            source,
        })?;
        let gz = flate2::read::GzDecoder::new(tgz);
        let mut ar = tar::Archive::new(gz);
        // tar crate rejects `..`/absolute members by default (no
        // set_overwrite/preserve escape), so extraction stays within `dest`.
        ar.unpack(&dest).map_err(|source| HelmRepoFetchError::Io {
            path: dest.clone(),
            source,
        })?;
    }
    Ok(Fetched {
        root_dir: root,
        digest,
        version: String::new(),
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
    let index_bytes = http_get(&client, &index_url, opts.auth)?;
    let index = parse_index(&index_bytes)?;
    let (version, tgz_url) = select_version(&index, chart, version_req)?;
    let abs_url = resolve_tarball_url(repo, &tgz_url);
    let tgz = http_get(&client, &abs_url, opts.auth)?;

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
    resp.bytes()
        .map(|b| b.to_vec())
        .map_err(|e| HelmRepoFetchError::Http {
            url: url.to_string(),
            detail: e.to_string(),
        })
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
            resolve_tarball_url(repo, "https://cdn.example.com/temporal-0.62.0.tgz"),
            "https://cdn.example.com/temporal-0.62.0.tgz"
        );
        assert_eq!(
            resolve_tarball_url(repo, "temporal-0.61.0.tgz"),
            "https://go.temporal.io/helm-charts/temporal-0.61.0.tgz"
        );
        assert_eq!(
            resolve_tarball_url("https://go.temporal.io/helm-charts/", "temporal-0.61.0.tgz"),
            "https://go.temporal.io/helm-charts/temporal-0.61.0.tgz"
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

                let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
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
