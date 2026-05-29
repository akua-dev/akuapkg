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
}
