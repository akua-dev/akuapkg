# HTTPS helm-repo dependencies — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a fourth `akua.toml` dependency source — classic HTTPS Helm repositories (`index.yaml` + `.tgz`) — content-pinned in `akua.lock` like git deps, with semver-range resolution.

**Architecture:** A new `repo`/`chart`/`version` dependency shape discriminates a `DependencySource::Helm`. A new `helm_repo_fetcher` module fetches `index.yaml`, selects a version (exact or semver range), downloads + extracts the `.tgz` into the content-addressed cache, and returns a tree sha256. `chart_resolver` gains a `resolve_helm` arm; the lockfile records `helm+<url>#<chart>` as the source ref and the tree sha256 as the digest. Determinism holds because `index.yaml` is consulted only at add/lock time; render uses the pinned digest.

**Tech Stack:** Rust (`akua-core`), `reqwest::blocking` + rustls (already an `oci-fetch` dependency), `flate2` + `tar` (extraction, already deps), `semver` crate (new — range resolution), `serde_yaml`/`serde_yml` (index.yaml parse, already a dep).

**Reference spec:** `docs/superpowers/specs/2026-05-29-https-helm-repo-deps-design.md`

---

## File structure

| File | Responsibility | Change |
|---|---|---|
| `crates/akua-core/src/mod_file.rs` | Dependency data model + validation | Modify: add `repo`/`chart` fields, `Helm` source variant, validation |
| `crates/akua-core/src/cli_contract.rs` | Stable error codes | Modify: add `E_MANIFEST_HELM_*` codes |
| `crates/akua-core/src/helm_repo_fetcher.rs` | Fetch `index.yaml`, select version, download+extract `.tgz`, digest | **Create** |
| `crates/akua-core/src/chart_resolver.rs` | Per-source resolution → `ResolvedChart` + lockfile fields | Modify: `ResolvedSource::Helm`, `resolve_helm`, `VendorKind::Helm` |
| `crates/akua-core/Cargo.toml` | deps + feature flag | Modify: add `semver`, `helm-fetch` feature |
| `crates/akua-core/src/lib.rs` | module registration | Modify: `mod helm_repo_fetcher;` |
| `crates/akuapkg-cli/src/verbs/add.rs` | `akuapkg add` helm form | Modify (Task 8) |
| `crates/akua-napi/src/lib.rs`, `packages/sdk/src/mod.ts` | SDK surface | Modify (Task 8) |
| `docs/lockfile-format.md`, `docs/package-format.md`, `docs/cli.md` | docs | Modify (Task 9) |

All `akua-core` tests run with: `cargo test -p akua-core --lib --features helm-fetch,oci-fetch`.

---

## Task 1: Cargo deps + feature flag

**Files:**
- Modify: `crates/akua-core/Cargo.toml`

- [ ] **Step 1: Add the `semver` dependency and `helm-fetch` feature**

In `[dependencies]` add (alphabetically near other crates):
```toml
semver = { version = "1", optional = true }
```

In `[features]`, add a `helm-fetch` feature mirroring `oci-fetch`'s transport deps plus semver:
```toml
helm-fetch = ["dep:reqwest", "dep:flate2", "dep:tar", "dep:semver"]
```
(Find the existing `oci-fetch = ["dep:reqwest", "dep:flate2", "dep:tar"]` line and add `helm-fetch` directly below it.)

- [ ] **Step 2: Verify it resolves**

Run: `cargo build -p akua-core --features helm-fetch`
Expected: builds (semver downloaded). No code uses it yet — that's fine.

- [ ] **Step 3: Commit**

```bash
git add crates/akua-core/Cargo.toml Cargo.lock
git commit -m "build(akua-core): add semver dep + helm-fetch feature"
```

---

## Task 2: Manifest error codes

**Files:**
- Modify: `crates/akua-core/src/cli_contract.rs`

- [ ] **Step 1: Add the error-code constants**

Find the `codes` module (grep `E_MANIFEST_GIT_USERINFO`). Add next to it:
```rust
pub const E_MANIFEST_HELM_MISSING_CHART: &str = "E_MANIFEST_HELM_MISSING_CHART";
pub const E_MANIFEST_HELM_MISSING_VERSION: &str = "E_MANIFEST_HELM_MISSING_VERSION";
pub const E_MANIFEST_HELM_USERINFO: &str = "E_MANIFEST_HELM_USERINFO";
```

- [ ] **Step 2: Verify**

Run: `cargo build -p akua-core`
Expected: builds.

- [ ] **Step 3: Commit**

```bash
git add crates/akua-core/src/cli_contract.rs
git commit -m "feat(cli-contract): error codes for helm-repo dep validation"
```

---

## Task 3: Manifest data model — the `Helm` source

**Files:**
- Modify: `crates/akua-core/src/mod_file.rs`

- [ ] **Step 1: Write failing tests for source discrimination + validation**

Add to `mod_file.rs`'s `#[cfg(test)] mod tests`:
```rust
#[test]
fn repo_dep_is_helm_source() {
    let dep = Dependency {
        repo: Some("https://go.temporal.io/helm-charts".into()),
        chart: Some("temporal".into()),
        version: Some("0.62.0".into()),
        ..Default::default()
    };
    assert_eq!(dep.source(), Some(DependencySource::Helm));
    dep.validate("temporal").expect("valid helm dep");
    match dep.spec() {
        DependencySpec::Helm { repo, chart, version } => {
            assert_eq!(repo, "https://go.temporal.io/helm-charts");
            assert_eq!(chart, "temporal");
            assert_eq!(version, "0.62.0");
        }
        other => panic!("expected Helm, got {other:?}"),
    }
}

#[test]
fn helm_dep_requires_chart_and_version() {
    let no_chart = Dependency {
        repo: Some("https://r".into()),
        version: Some("1.0.0".into()),
        ..Default::default()
    };
    assert!(matches!(
        no_chart.validate("x"),
        Err(ManifestError::HelmMissingChart { .. })
    ));
    let no_version = Dependency {
        repo: Some("https://r".into()),
        chart: Some("c".into()),
        ..Default::default()
    };
    assert!(matches!(
        no_version.validate("x"),
        Err(ManifestError::HelmMissingVersion { .. })
    ));
}

#[test]
fn helm_repo_url_rejects_userinfo() {
    let dep = Dependency {
        repo: Some("https://user:pass@r".into()),
        chart: Some("c".into()),
        version: Some("1.0.0".into()),
        ..Default::default()
    };
    assert!(matches!(
        dep.validate("x"),
        Err(ManifestError::HelmUrlHasUserInfo { .. })
    ));
}

#[test]
fn repo_is_mutually_exclusive_with_other_sources() {
    let dep = Dependency {
        repo: Some("https://r".into()),
        oci: Some("oci://x".into()),
        chart: Some("c".into()),
        version: Some("1.0.0".into()),
        ..Default::default()
    };
    assert_eq!(dep.source(), None);
    assert!(matches!(
        dep.validate("x"),
        Err(ManifestError::AmbiguousSource { .. })
    ));
}
```

- [ ] **Step 2: Run tests, verify they fail to compile**

Run: `cargo test -p akua-core --lib mod_file::tests::repo_dep_is_helm_source`
Expected: compile error — `repo`/`chart` fields and `DependencySource::Helm` don't exist yet.

- [ ] **Step 3: Add the fields to `Dependency`**

After the `path` field (around line 99):
```rust
    /// HTTPS Helm-repo URL (the `index.yaml` lives at `<repo>/index.yaml`).
    /// Exclusive with `oci`, `git`, `path`. Pairs with `chart`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,

    /// Chart entry name within a `repo`'s `index.yaml`. Required for
    /// `repo` deps; unused by other sources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chart: Option<String>,
```

- [ ] **Step 4: Add the `Helm` variants**

In `DependencySource`:
```rust
pub enum DependencySource {
    Oci,
    Git,
    Path,
    Helm,
}
```
In `DependencySpec`:
```rust
    Helm {
        repo: &'a str,
        chart: &'a str,
        version: &'a str,
    },
```

- [ ] **Step 5: Update `source()`**

The current 3-bool match must become a 4-bool match. Replace the body of `source()`:
```rust
    pub fn source(&self) -> Option<DependencySource> {
        match (
            self.oci.is_some(),
            self.git.is_some(),
            self.path.is_some(),
            self.repo.is_some(),
        ) {
            (true, false, false, false) => Some(DependencySource::Oci),
            (false, true, false, false) => Some(DependencySource::Git),
            (false, false, true, false) => Some(DependencySource::Path),
            (false, false, false, true) => Some(DependencySource::Helm),
            _ => None,
        }
    }
```

- [ ] **Step 6: Update `spec()`**

Add a `Helm` arm. The current `spec()` matches a 3-tuple; extend to include `repo`:
```rust
    pub fn spec(&self) -> DependencySpec<'_> {
        match (
            self.path.as_deref(),
            self.oci.as_deref(),
            self.git.as_deref(),
            self.repo.as_deref(),
        ) {
            (Some(declared), None, None, None) => DependencySpec::Path { declared },
            (None, Some(oci), None, None) => DependencySpec::Oci {
                oci,
                version: self.version.as_deref().expect(
                    "Dependency::spec called on an unvalidated manifest — call validate() first",
                ),
            },
            (None, None, Some(git), None) => DependencySpec::Git {
                git,
                tag: self.tag.as_deref(),
                rev: self.rev.as_deref(),
            },
            (None, None, None, Some(repo)) => DependencySpec::Helm {
                repo,
                chart: self.chart.as_deref().expect(
                    "Dependency::spec called on an unvalidated manifest — call validate() first",
                ),
                version: self.version.as_deref().expect(
                    "Dependency::spec called on an unvalidated manifest — call validate() first",
                ),
            },
            _ => unreachable!(
                "Dependency::spec called on an unvalidated manifest — call validate() first"
            ),
        }
    }
```

- [ ] **Step 7: Add the `ManifestError` variants**

Next to `GitUrlHasUserInfo`:
```rust
    #[error("dependency `{name}`: helm-repo dep requires a `chart`")]
    HelmMissingChart { name: String },

    #[error("dependency `{name}`: helm-repo dep requires a `version`")]
    HelmMissingVersion { name: String },

    #[error(
        "dependency `{name}`: repo URL must not embed credentials (`user:pass@`). \
         Pass credentials via the SDK `auth` parameter or the CLI `--auth` flag."
    )]
    HelmUrlHasUserInfo { name: String },
```

- [ ] **Step 8: Map the new errors in `structured_code()`**

In `ManifestError::structured_code`, add arms before the `_ =>` catch-all:
```rust
            ManifestError::HelmMissingChart { .. } => codes::E_MANIFEST_HELM_MISSING_CHART,
            ManifestError::HelmMissingVersion { .. } => codes::E_MANIFEST_HELM_MISSING_VERSION,
            ManifestError::HelmUrlHasUserInfo { .. } => codes::E_MANIFEST_HELM_USERINFO,
```

- [ ] **Step 9: Extend `validate()`**

Add a `Helm` arm to the `match source` block in `validate()`:
```rust
            DependencySource::Helm if self.chart.is_none() => {
                Err(ManifestError::HelmMissingChart { name: name.to_string() })
            }
            DependencySource::Helm if self.version.is_none() => {
                Err(ManifestError::HelmMissingVersion { name: name.to_string() })
            }
            DependencySource::Helm
                if crate::host_auth::url_has_userinfo(
                    self.repo.as_deref().expect("repo source set"),
                ) =>
            {
                Err(ManifestError::HelmUrlHasUserInfo { name: name.to_string() })
            }
```

- [ ] **Step 10: Run the Task-3 tests, verify they pass**

Run: `cargo test -p akua-core --lib mod_file::tests`
Expected: all pass, including the 4 new tests. Fix any non-exhaustive-match errors the compiler flags in other files (e.g. a `match dep.source()` elsewhere) by adding `DependencySource::Helm` arms — grep `DependencySource::Path =>` to find them.

- [ ] **Step 11: Commit**

```bash
git add crates/akua-core/src/mod_file.rs
git commit -m "feat(manifest): repo/chart helm-repo dependency source + validation"
```

---

## Task 4: `helm_repo_fetcher` — index.yaml types + parse

**Files:**
- Create: `crates/akua-core/src/helm_repo_fetcher.rs`
- Modify: `crates/akua-core/src/lib.rs`

- [ ] **Step 1: Register the module (feature-gated)**

In `lib.rs`, near the `#[cfg(feature = "oci-fetch")] mod oci_fetcher;` lines, add:
```rust
#[cfg(feature = "helm-fetch")]
pub mod helm_repo_fetcher;
```

- [ ] **Step 2: Write the failing index-parse test**

Create `crates/akua-core/src/helm_repo_fetcher.rs`:
```rust
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
    NoMatchingVersion { chart: String, req: String, available: String },
    #[error("chart `{chart}`@`{version}` has no download URL in the index")]
    NoUrl { chart: String, version: String },
    #[error("invalid version `{version}` in index for `{chart}`: {detail}")]
    BadVersion { chart: String, version: String, detail: String },
    #[error("invalid version requirement `{req}`: {detail}")]
    BadRequirement { req: String, detail: String },
    #[error("fetching {url}: {detail}")]
    Http { url: String, detail: String },
    #[error("digest mismatch for `{chart}`: expected {expected}, got {actual}")]
    DigestMismatch { chart: String, expected: String, actual: String },
    #[error("offline and `{chart}` is not in the cache — run `akuapkg add` online first")]
    OfflineCacheMiss { chart: String },
    #[error("io error at {path}: {source}")]
    Io { path: std::path::PathBuf, #[source] source: std::io::Error },
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
```

- [ ] **Step 3: Run, verify pass**

Run: `cargo test -p akua-core --lib --features helm-fetch helm_repo_fetcher::tests::parses_index_entries`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/akua-core/src/helm_repo_fetcher.rs crates/akua-core/src/lib.rs
git commit -m "feat(helm-repo): index.yaml types + parser"
```

---

## Task 5: Version selection (exact + semver range)

**Files:**
- Modify: `crates/akua-core/src/helm_repo_fetcher.rs`

- [ ] **Step 1: Write the failing selection test**

Add to the `tests` module:
```rust
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
```

- [ ] **Step 2: Run, verify fails to compile (`select_version` missing)**

Run: `cargo test -p akua-core --lib --features helm-fetch helm_repo_fetcher::tests::selects_exact_version`
Expected: compile error — `select_version` not found.

- [ ] **Step 3: Implement `select_version`**

Add to `helm_repo_fetcher.rs` (above the tests module):
```rust
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
        .ok_or_else(|| HelmRepoFetchError::ChartNotFound { chart: chart.to_string() })?;

    let req = semver::VersionReq::parse(version_req).map_err(|e| {
        HelmRepoFetchError::BadRequirement { req: version_req.to_string(), detail: e.to_string() }
    })?;

    let mut best: Option<(semver::Version, &IndexEntry)> = None;
    for entry in entries {
        let ver = semver::Version::parse(&entry.version).map_err(|e| {
            HelmRepoFetchError::BadVersion {
                chart: chart.to_string(),
                version: entry.version.clone(),
                detail: e.to_string(),
            }
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
```

Note: the `semver` crate treats a bare `0.62.0` as `^0.62.0` (caret). For Helm's exact-pin semantics, callers that wrote an exact version still match it (it's the highest and only equal version in range). This is acceptable: `akua.lock` records the resolved exact version, so re-resolution is reproducible regardless of caret widening, and the digest pin is authoritative.

- [ ] **Step 4: Run, verify pass**

Run: `cargo test -p akua-core --lib --features helm-fetch helm_repo_fetcher::tests`
Expected: all selection tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/akua-core/src/helm_repo_fetcher.rs
git commit -m "feat(helm-repo): semver version selection against index"
```

---

## Task 6: Tarball URL resolution (relative vs absolute)

**Files:**
- Modify: `crates/akua-core/src/helm_repo_fetcher.rs`

- [ ] **Step 1: Write the failing test**

```rust
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
```

- [ ] **Step 2: Run, verify fails to compile**

Run: `cargo test -p akua-core --lib --features helm-fetch helm_repo_fetcher::tests::resolves_relative_and_absolute_urls`
Expected: compile error — `resolve_tarball_url` not found.

- [ ] **Step 3: Implement**

```rust
/// Resolve a tarball URL from `index.yaml` against the repo base.
/// Absolute (`http://`/`https://`) URLs pass through; relative ones
/// are joined to `<repo>/`.
pub fn resolve_tarball_url(repo: &str, url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_string();
    }
    format!("{}/{}", repo.trim_end_matches('/'), url.trim_start_matches('/'))
}
```

- [ ] **Step 4: Run, verify pass**

Run: `cargo test -p akua-core --lib --features helm-fetch helm_repo_fetcher::tests::resolves_relative_and_absolute_urls`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/akua-core/src/helm_repo_fetcher.rs
git commit -m "feat(helm-repo): resolve relative/absolute tarball URLs"
```

---

## Task 7: End-to-end fetch (download + extract + digest + offline)

**Files:**
- Modify: `crates/akua-core/src/helm_repo_fetcher.rs`

This task wires the HTTP GETs, extraction, and the cache. The download/extract/hash mirror `oci_fetcher`. Tests use a `file://`-free approach: a local HTTP server is heavy, so the unit test exercises `fetch_from_cache` + `extract_and_hash` directly, and the live GET path is covered by the Task 11 integration golden.

- [ ] **Step 1: Write the failing extract+hash test**

```rust
#[test]
fn extract_and_hash_unpacks_chart_and_pins_digest() {
    use std::io::Write;
    // Build a minimal chart .tgz in memory: Chart.yaml + a template.
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    {
        let mut tar = tar::Builder::new(&mut gz);
        let chart_yaml = b"apiVersion: v2\nname: demo\nversion: 0.1.0\n";
        let mut h = tar::Header::new_gnu();
        h.set_size(chart_yaml.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        tar.append_data(&mut h, "demo/Chart.yaml", &chart_yaml[..]).unwrap();
        tar.finish().unwrap();
    }
    let tgz = gz.finish().unwrap();

    let cache = tempfile::tempdir().unwrap();
    let first = extract_and_hash(&tgz, cache.path(), "demo").expect("extract");
    assert!(first.root_dir.join("Chart.yaml").is_file());
    assert!(first.digest.starts_with("sha256:"));

    // Deterministic: same bytes → same digest, and it lands in the cache.
    let cached = fetch_from_cache(cache.path(), &first.digest).expect("cached");
    assert_eq!(cached.digest, first.digest);
    assert!(cached.root_dir.join("Chart.yaml").is_file());
}
```

- [ ] **Step 2: Run, verify fails to compile**

Run: `cargo test -p akua-core --lib --features helm-fetch helm_repo_fetcher::tests::extract_and_hash_unpacks_chart_and_pins_digest`
Expected: compile error — `extract_and_hash` / `fetch_from_cache` / `Fetched` not found.

- [ ] **Step 3: Implement the fetch surface**

Add to `helm_repo_fetcher.rs`. The digest is the sha256 of the **tarball bytes** (content-address of what the repo served), and the unpacked tree lands under `<cache>/<digest-hex>/`:
```rust
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
    Ok(Fetched { root_dir: root, digest, version: String::new() })
}

/// Offline path: return the cached unpack for a pinned digest, if present.
pub fn fetch_from_cache(cache_root: &Path, digest: &str) -> Option<Fetched> {
    let dest = cache_dir_for(cache_root, digest);
    // The chart dir name isn't known here; find the single subdir holding Chart.yaml.
    let entry = std::fs::read_dir(&dest).ok()?.flatten().find(|e| {
        e.path().join("Chart.yaml").is_file()
    })?;
    Some(Fetched {
        root_dir: entry.path(),
        digest: digest.to_string(),
        version: String::new(),
    })
}
```
Confirm `sha2` is already a dependency (grep `sha2` in `crates/akua-core/Cargo.toml`; OCI digesting uses it). If absent, add `sha2 = "0.10"` under the `helm-fetch` feature deps.

- [ ] **Step 4: Run, verify pass**

Run: `cargo test -p akua-core --lib --features helm-fetch helm_repo_fetcher::tests::extract_and_hash_unpacks_chart_and_pins_digest`
Expected: PASS.

- [ ] **Step 5: Add the online `fetch` entrypoint (no new test — covered by Task 11 golden)**

```rust
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
        .map_err(|e| HelmRepoFetchError::Http { url: repo.to_string(), detail: e.to_string() })?;

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
    let resp = req
        .send()
        .map_err(|e| HelmRepoFetchError::Http { url: url.to_string(), detail: e.to_string() })?;
    if !resp.status().is_success() {
        return Err(HelmRepoFetchError::Http {
            url: url.to_string(),
            detail: format!("HTTP {}", resp.status()),
        });
    }
    resp.bytes()
        .map(|b| b.to_vec())
        .map_err(|e| HelmRepoFetchError::Http { url: url.to_string(), detail: e.to_string() })
}
```
Note: confirm the `BasicAuth` field names (`username`/`password`) by grepping `pub struct BasicAuth` in `host_auth.rs`; adjust the `basic_auth(...)` call if they differ.

- [ ] **Step 6: Run the whole module + build with feature**

Run: `cargo test -p akua-core --lib --features helm-fetch helm_repo_fetcher::tests`
Then: `cargo build -p akua-core --features helm-fetch`
Expected: tests PASS, build clean.

- [ ] **Step 7: Commit**

```bash
git add crates/akua-core/src/helm_repo_fetcher.rs crates/akua-core/Cargo.toml
git commit -m "feat(helm-repo): http fetch + extract + content-digest pin + offline cache"
```

---

## Task 8: Resolver + lockfile wiring

**Files:**
- Modify: `crates/akua-core/src/chart_resolver.rs`

- [ ] **Step 1: Write the failing resolver test**

Add to `chart_resolver.rs` tests (grep an existing resolver test for the helper `write_manifest`/temp-workspace pattern and mirror it). The test resolves a `repo` dep from a pre-populated cache (offline) so no network is needed:
```rust
#[test]
#[cfg(feature = "helm-fetch")]
fn resolves_helm_repo_dep_from_cache() {
    use crate::helm_repo_fetcher;
    // Prime the cache with a demo chart tarball.
    let cache = tempfile::tempdir().unwrap();
    let tgz = /* build minimal demo .tgz — reuse the helper from Task 7 test */
        crate::helm_repo_fetcher::tests::demo_tgz();
    let primed = helm_repo_fetcher::extract_and_hash(&tgz, cache.path(), "demo").unwrap();

    let ws = tempfile::tempdir().unwrap();
    std::fs::write(
        ws.path().join("akua.toml"),
        format!(
            "[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"akua.dev/v1alpha1\"\n\
             [dependencies.demo]\nrepo = \"https://example.com/charts\"\nchart = \"demo\"\nversion = \"0.1.0\"\n"
        ),
    ).unwrap();
    let manifest = AkuaManifest::load(ws.path()).unwrap();

    let mut expected = std::collections::BTreeMap::new();
    expected.insert("demo".to_string(), primed.digest.clone());
    let opts = ResolverOptions {
        cache_root: Some(cache.path().to_path_buf()),
        offline: true,
        expected_digests: expected,
        ..ResolverOptions::default()
    };
    let resolved = resolve_with_options(&manifest, ws.path(), &opts).unwrap();
    let chart = resolved.entries.get("demo").unwrap();
    assert!(chart.abs_path.join("Chart.yaml").is_file());
    assert_eq!(chart.source.kind_str(), "helm");
}
```
To support this test, promote the Task-7 `demo_tgz` builder into a `#[cfg(test)] pub(crate) fn demo_tgz() -> Vec<u8>` in `helm_repo_fetcher::tests` and reference it. Confirm `kind_str()`/`source` accessor names against `ResolvedChart` (grep `impl ResolvedChart`); adjust the final assert to the real accessor.

- [ ] **Step 2: Run, verify fails to compile**

Run: `cargo test -p akua-core --lib --features helm-fetch,oci-fetch chart_resolver::tests::resolves_helm_repo_dep_from_cache`
Expected: compile error — `ResolvedSource::Helm` / resolve arm missing.

- [ ] **Step 3: Add `ResolvedSource::Helm`**

In the `ResolvedSource` enum:
```rust
    /// Helm-repo-sourced dep, fetched via `helm_repo_fetcher`. `digest`
    /// is the `.tgz` tree sha256; lockfile stores it under `digest`.
    Helm {
        repo: String,
        chart: String,
        version: String,
        digest: String,
    },
```

- [ ] **Step 4: Extend `kind_str()` / `to_locked_fields()`**

In `kind_str()` (the `match` returning `"path"`/`"oci"`/`"git"`), add:
```rust
            ResolvedSource::Helm { .. } => "helm",
```
In `to_locked_fields()`, add an arm. Encode the chart in the source ref fragment so no `LockedPackage` schema change is needed:
```rust
            ResolvedSource::Helm { repo, chart, version: _, digest } => (
                format!("helm+{repo}#{chart}"),
                digest.clone(),
                None,
            ),
```

- [ ] **Step 5: Add `VendorKind::Helm` + the `resolve_helm` arm**

In `VendorKind`:
```rust
    Helm { repo: &'a str, chart: &'a str, version: String },
```
In `resolve_with_options`, in the `vendor_kind` match:
```rust
            DependencySpec::Helm { repo, chart, version } => VendorKind::Helm {
                repo,
                chart,
                version: version.to_string(),
            },
```
In the same fn's `resolve_from_vendor` placeholder match, add:
```rust
        VendorKind::Helm { repo, chart, version } => ResolvedSource::Helm {
            repo: (*repo).to_string(),
            chart: (*chart).to_string(),
            version: version.clone(),
            digest: String::new(),
        },
```
In the final `match spec` dispatch:
```rust
            DependencySpec::Helm { repo, chart, version } => {
                resolve_helm(name, repo, chart, version, opts)?
            }
```

- [ ] **Step 6: Implement `resolve_helm`**

Mirror `resolve_oci`. Add near it (feature-gate the whole fn with `#[cfg(feature = "helm-fetch")]`, and add a `#[cfg(not(feature = "helm-fetch"))]` stub that returns `ChartResolveError::UnsupportedSource { kind: DependencySource::Helm, reason: "built without helm-fetch" }`):
```rust
#[cfg(feature = "helm-fetch")]
fn resolve_helm(
    name: &str,
    repo: &str,
    chart: &str,
    version: &str,
    opts: &ResolverOptions,
) -> Result<ResolvedChart, ChartResolveError> {
    let cache_root = opts.cache_root.clone().unwrap_or_else(default_helm_cache_root);
    let expected = opts.expected_digests.get(name).map(String::as_str);

    let fetched = if opts.offline {
        let digest = expected.ok_or_else(|| ChartResolveError::UnsupportedSource {
            name: name.to_string(),
            kind: DependencySource::Helm,
            reason: "offline mode needs a lockfile-pinned digest — run `akuapkg add` first",
        })?;
        crate::helm_repo_fetcher::fetch_from_cache(&cache_root, digest).ok_or_else(|| {
            ChartResolveError::UnsupportedSource {
                name: name.to_string(),
                kind: DependencySource::Helm,
                reason: "offline and chart not cached — run `akuapkg add` online first",
            }
        })?
    } else {
        let auth = opts.host_auth.as_ref();
        let fetch_opts = crate::helm_repo_fetcher::FetchOpts { expected_digest: expected, auth };
        crate::helm_repo_fetcher::fetch(repo, chart, version, &cache_root, &fetch_opts)
            .map_err(|source| ChartResolveError::HelmFetch { name: name.to_string(), source })?
    };

    // Offline `fetch_from_cache` doesn't know the resolved version; fall back to the declared one.
    let resolved_version = if fetched.version.is_empty() { version.to_string() } else { fetched.version.clone() };
    resolve_path(
        name,
        &fetched.root_dir.to_string_lossy(),
        // helm cache roots are absolute; resolve_path treats this as already-materialized.
        fetched.root_dir.parent().unwrap_or(&fetched.root_dir),
        ResolvedSource::Helm {
            repo: repo.to_string(),
            chart: chart.to_string(),
            version: resolved_version,
            digest: fetched.digest.clone(),
        },
        PathOrigin::Internal,
    )
}

#[cfg(feature = "helm-fetch")]
fn default_helm_cache_root() -> PathBuf {
    default_oci_cache_root()
        .parent()
        .map(|p| p.join("helm"))
        .unwrap_or_else(|| PathBuf::from(".akua-cache/helm"))
}
```
Confirm `resolve_path`'s exact parameter list (grep `fn resolve_path`) and the `ResolverOptions.host_auth` field name (grep `host_auth` in `ResolverOptions`; if absent, add `pub host_auth: Option<crate::host_auth::HostAuthMap>` with a `Default` of `None` and thread it from the CLI like `cosign_public_key_pem`). Adjust the `resolve_path` call to the real signature — it hashes the dir tree and assigns the final `sha256` to `abs_path`.

- [ ] **Step 7: Add the `HelmFetch` error variant**

In `ChartResolveError` (next to `OciFetch`):
```rust
    #[cfg(feature = "helm-fetch")]
    #[error("chart `{name}`: helm-repo fetch failed: {source}")]
    HelmFetch {
        name: String,
        source: crate::helm_repo_fetcher::HelmRepoFetchError,
    },
```

- [ ] **Step 8: Run the resolver test + fix exhaustiveness**

Run: `cargo test -p akua-core --lib --features helm-fetch,oci-fetch chart_resolver::tests::resolves_helm_repo_dep_from_cache`
Expected: PASS. Fix any non-exhaustive `match` on `ResolvedSource`/`DependencySource`/`VendorKind` the compiler flags (lockfile writer, `vendor` verb) by adding `Helm` arms.

- [ ] **Step 9: Commit**

```bash
git add crates/akua-core/src/chart_resolver.rs
git commit -m "feat(helm-repo): resolver arm + lockfile source ref"
```

---

## Task 9: Lockfile round-trip + full akua-core suite

**Files:**
- Modify: `crates/akua-core/src/lock_file.rs` (test only, unless a `Helm` arm is needed in a decoder)

- [ ] **Step 1: Write a lockfile round-trip test for a helm source**

Add to `lock_file.rs` tests:
```rust
#[test]
fn helm_source_round_trips() {
    let pkg = LockedPackage {
        name: "temporal".into(),
        version: "0.62.0".into(),
        source: "helm+https://go.temporal.io/helm-charts#temporal".into(),
        digest: "sha256:abc123".into(),
        vendor_digest: None,
        signature: None,
        dependencies: vec![],
        attestation: None,
    };
    let lock = LockFile { version: CURRENT_VERSION, package: vec![pkg.clone()] };
    let toml = toml::to_string(&lock).unwrap();
    let back: LockFile = toml::from_str(&toml).unwrap();
    assert_eq!(back.package[0], pkg);
    assert!(toml.contains("helm+https://go.temporal.io/helm-charts#temporal"));
}
```
Adjust `LockedPackage`/`LockFile` field set to the real struct (Task notes: it also has `vendor_digest`, `signature`, `dependencies`, `attestation` — all default/skip).

- [ ] **Step 2: Run, verify pass**

Run: `cargo test -p akua-core --lib lock_file::tests::helm_source_round_trips`
Expected: PASS (no production change needed — `source` is a free-form string).

- [ ] **Step 3: Run the FULL akua-core suite, all features**

Run: `cargo test -p akua-core --lib --features helm-fetch,oci-fetch,git-fetch,cosign-verify`
Expected: all green (the prior 400 + the new tests).

- [ ] **Step 4: fmt + clippy**

Run: `cargo fmt -p akua-core && cargo clippy -p akua-core --features helm-fetch,oci-fetch -- -D warnings`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/akua-core/src/lock_file.rs
git commit -m "test(lockfile): helm-repo source round-trip"
```

---

## Task 10: CLI `akuapkg add` + SDK surface

**Files:**
- Modify: `crates/akuapkg-cli/src/verbs/add.rs`
- Modify: `crates/akuapkg-cli/src/main.rs` (clap flags)
- Modify: `crates/akua-napi/src/lib.rs`, `packages/sdk/src/mod.ts`

- [ ] **Step 1: Inspect the existing `add` flag wiring**

Read `crates/akuapkg-cli/src/verbs/add.rs` and the `Add` clap struct in `main.rs`. Note how `--oci`/`--git`/`--path`/`--version`/`--tag`/`--rev` map onto a `Dependency`.

- [ ] **Step 2: Write the failing CLI test**

In `add.rs` tests (mirror an existing `adds_oci_dep_with_version` test):
```rust
#[test]
fn adds_helm_repo_dep() {
    // Arrange a temp workspace with an akua.toml, call the add logic with
    // repo/chart/version, assert the [dependencies.temporal] table has
    // repo + chart + version and no oci/git/path.
}
```
Fill the body by copying the OCI add test and swapping the asserted fields to `repo`/`chart`/`version`.

- [ ] **Step 3: Run, verify fails**

Run: `cargo test -p akuapkg-cli adds_helm_repo_dep`
Expected: FAIL/compile error — no `--repo`/`--chart` flags.

- [ ] **Step 4: Add `--repo` + `--chart` clap flags and map them**

Add `repo: Option<String>` and `chart: Option<String>` to the `Add` args struct; in `add::run`, when `repo` is set, build a `Dependency { repo, chart, version, ..Default }` and run `akuapkg lock` resolution to pin the digest (reuse the existing post-add lock path the OCI form uses).

- [ ] **Step 5: Run, verify pass**

Run: `cargo test -p akuapkg-cli adds_helm_repo_dep`
Expected: PASS.

- [ ] **Step 6: Mirror in napi + SDK**

Add `repo?`/`chart?` to the napi `add` shim and `Akua.add()` options in `packages/sdk/src/mod.ts`, routing through `loadNapi()`/`callNapi` like the existing fields. Run `bun test` in `packages/sdk` if present.

- [ ] **Step 7: Commit**

```bash
git add crates/akuapkg-cli crates/akua-napi packages/sdk
git commit -m "feat(add): akuapkg add --repo --chart helm-repo form + SDK"
```

---

## Task 11: Integration golden + end-to-end render

**Files:**
- Create: `crates/akuapkg-cli/tests/fixtures/helm-repo/` (a fixture repo: `index.yaml` + a small `.tgz`)
- Create: `crates/akuapkg-cli/tests/examples_helm_repo.rs`

- [ ] **Step 1: Build a fixture helm repo**

Create a tiny chart and package it, plus an `index.yaml` pointing at it with a relative URL. Commit both the chart source and the generated `.tgz` + `index.yaml` under the fixtures dir. (A served HTTP repo isn't needed if the integration test points the resolver's `cache_root` at a pre-primed cache and runs offline — preferred for determinism. If exercising the live GET, run a `std::net::TcpListener` mini-server in the test serving the fixture files.)

- [ ] **Step 2: Write the integration test (offline, pre-primed cache)**

A Package with `[dependencies.demo] repo/chart/version`, an `akua.lock` pinning the fixture digest, rendered with `cache_root` → the primed cache and `offline=true`; assert the rendered output contains the chart's resource and is byte-stable across two runs.

- [ ] **Step 3: Run**

Run: `cargo test -p akuapkg-cli --features helm-fetch,oci-fetch examples_helm_repo`
Expected: PASS, deterministic.

- [ ] **Step 4: Real-world smoke (manual, documented in the test file as a comment)**

Build the binary (`task build:render-worker && task release:local`), point a throwaway Package at temporal's real repo (`repo = "https://go.temporal.io/helm-charts"`, `chart = "temporal"`, `version = ">=0.60,<0.63"`), `akuapkg add` then `akuapkg render`, confirm 55 manifests and a populated `akua.lock`. Delete the throwaway. (Not a CI test — network-dependent.)

- [ ] **Step 5: Commit**

```bash
git add crates/akuapkg-cli/tests
git commit -m "test(helm-repo): integration golden for repo-sourced chart render"
```

---

## Task 12: Docs + verb-count/contract sync

**Files:**
- Modify: `docs/lockfile-format.md`, `docs/package-format.md`, `docs/cli.md`, `CHANGELOG.md`

- [ ] **Step 1: Document the `repo`/`chart` source**

In `docs/package-format.md` and `docs/cli.md`, add `repo` + `chart` to the dependency-source table with the temporal example. In `docs/lockfile-format.md`, document the `helm+<url>#<chart>` source ref + `sha256:` digest. Add an `akuapkg add --repo` example to the `add` section.

- [ ] **Step 2: CHANGELOG entry**

Under `## [Unreleased]` → `### Added`:
```markdown
- **HTTPS helm-repo dependency source** ([helm_repo_fetcher.rs](crates/akua-core/src/helm_repo_fetcher.rs)). `akua.toml` deps can now name a classic Helm repository (`repo` + `chart` + `version`, exact or semver range) alongside `oci`/`git`/`path`. Resolved against the repo's `index.yaml` at add/lock time, content-pinned by `.tgz` sha256 in `akua.lock`, rendered deterministically offline. Private repos use the existing host-keyed `--auth`.
```

- [ ] **Step 3: Verify docs build / link check if a task exists**

Run: `mise exec -- task release:validate` (or the docs link checker if present).
Expected: green.

- [ ] **Step 4: Commit**

```bash
git add docs CHANGELOG.md
git commit -m "docs(helm-repo): document repo/chart dependency source"
```

---

## Self-review

**Spec coverage:**
- Authoring shape (`repo`/`chart`/`version`) → Task 3. ✓
- Protocol (index.yaml, semver select, tarball URL, download, auth, offline) → Tasks 4–7. ✓
- Trust model (content-pin sha256, verify-on-pull) → Task 7 (`expected_digest` check) + Task 8 (lockfile). ✓
- Determinism (index only at add/lock; render from digest) → Task 8 offline path + Task 11 golden. ✓
- Data model + validation → Task 3; error codes → Task 2. ✓
- Resolver + vendor-first → Task 8. ✓
- Lockfile → Task 8 (`to_locked_fields`) + Task 9 (round-trip). ✓
- CLI/SDK contract → Task 10. ✓
- Security (userinfo rejection, path-escape on extract, replace stripping) → Task 3 validation + Task 7 `tar::unpack` note. ✓
- Semver ranges → Task 5. ✓
- Docs → Task 12. ✓

**Placeholder scan:** Tasks 10–11 leave two test bodies described rather than fully written (the `akuapkg add` test and the integration fixture) because they must mirror existing CLI test scaffolding whose exact helpers aren't quoted here; each step names the existing test to copy and the exact fields to assert. All `akua-core` code steps contain complete code.

**Type consistency:** `Fetched{root_dir,digest,version}`, `FetchOpts{expected_digest,auth}`, `HelmRepoFetchError`, `ResolvedSource::Helm{repo,chart,version,digest}`, `DependencySpec::Helm{repo,chart,version}`, and `select_version → (String,String)` are used consistently across Tasks 4–9. Accessor/field names that must be confirmed against existing code are flagged inline (`BasicAuth` fields, `resolve_path` signature, `ResolverOptions.host_auth`, `ResolvedChart` accessor).

**Scope:** single subsystem (one dependency source), one plan.
