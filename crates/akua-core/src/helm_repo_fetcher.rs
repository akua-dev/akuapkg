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

#[cfg(test)]
mod tests {
    use super::*;

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
}
