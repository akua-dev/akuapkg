//! Workspace vendor-tree sync and inspection.
//!
//! The resolver already prefers `<workspace>/.akua/vendor/<name>/`
//! when it exists. This module gives that tree a first-class
//! maintenance surface:
//!
//! - `add` materializes a declared dependency into the vendor tree.
//! - `check` compares vendor bytes against what the manifest + lock
//!   know about and reports drift.
//! - `list` inventories the on-disk vendor trees, including orphans.
//!
//! The digest contract matches the rest of akua: SHA-256 over the
//! directory tree, with lexicographically sorted entries so byte
//! identity is stable across filesystems.
#![allow(clippy::result_large_err)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::cli_contract::{codes, ExitCode, StructuredError};
use crate::lock_file::{AkuaLock, LockLoadError, LockedPackage};
use crate::mod_file::{AkuaManifest, Dependency, DependencySource, ManifestLoadError};
#[cfg(feature = "oci-fetch")]
use crate::{cache_inventory, oci_auth, oci_fetcher};
#[cfg(feature = "git-fetch")]
use crate::{git_fetcher, git_fetcher::RefSpec};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VendorListEntry {
    pub name: String,
    pub path: PathBuf,
    pub digest: String,
    pub size_bytes: u64,
    pub orphan: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VendorListOutput {
    pub entries: Vec<VendorListEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VendorCheckEntry {
    pub name: String,
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_digest: Option<String>,
    pub orphan: bool,
    pub missing_vendor: bool,
    pub missing_source: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VendorCheckOutput {
    pub drift: bool,
    pub entries: Vec<VendorCheckEntry>,
    pub orphaned: Vec<String>,
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VendorAddOutput {
    pub name: String,
    pub source_kind: String,
    pub source_ref: String,
    pub path: PathBuf,
    pub digest: String,
    pub size_bytes: u64,
    pub wrote: bool,
    pub replaced: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum VendorError {
    #[error(transparent)]
    Manifest(#[from] ManifestLoadError),

    #[error(transparent)]
    Lock(#[from] LockLoadError),

    #[error("reading OCI credentials: {0}")]
    AuthConfig(String),

    #[error(
        "dep `{name}` is not declared in akua.toml; declare `path = \".akua/vendor/{name}\"` first"
    )]
    MissingDependency { name: String },

    #[error("source path `{path}` does not exist")]
    SourceMissing { path: PathBuf },

    #[error("i/o at `{}`: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[cfg(feature = "oci-fetch")]
    #[error("fetching OCI dep `{name}`: {source}")]
    OciFetch {
        name: String,
        #[source]
        source: oci_fetcher::OciFetchError,
    },

    #[cfg(feature = "git-fetch")]
    #[error("fetching git dep `{name}`: {source}")]
    GitFetch {
        name: String,
        #[source]
        source: git_fetcher::GitFetchError,
    },

    #[error("vendor tree drift detected")]
    Drift { output: VendorCheckOutput },
}

impl VendorError {
    pub fn to_structured(&self) -> StructuredError {
        match self {
            VendorError::Manifest(e) => e.to_structured(),
            VendorError::Lock(e) => e.to_structured(),
            VendorError::AuthConfig(detail) => {
                StructuredError::new(codes::E_IO, detail.clone()).with_default_docs()
            }
            VendorError::MissingDependency { .. } => {
                StructuredError::new(codes::E_VENDOR_DEP_MISSING, self.to_string())
                    .with_suggestion(
                        "declare `path = \".akua/vendor/<name>\"` in `akua.toml` first",
                    )
                    .with_default_docs()
            }
            VendorError::SourceMissing { path } => {
                StructuredError::new(codes::E_IO, self.to_string())
                    .with_path(path.display().to_string())
                    .with_default_docs()
            }
            VendorError::Io { path, source } => {
                StructuredError::new(codes::E_IO, source.to_string())
                    .with_path(path.display().to_string())
                    .with_default_docs()
            }
            #[cfg(feature = "oci-fetch")]
            VendorError::OciFetch { .. } => {
                StructuredError::new(codes::E_CHART_RESOLVE, self.to_string()).with_default_docs()
            }
            #[cfg(feature = "git-fetch")]
            VendorError::GitFetch { .. } => {
                StructuredError::new(codes::E_CHART_RESOLVE, self.to_string()).with_default_docs()
            }
            VendorError::Drift { .. } => {
                StructuredError::new(codes::E_VENDOR_DRIFT, self.to_string()).with_default_docs()
            }
        }
    }

    pub fn exit_code(&self) -> ExitCode {
        match self {
            VendorError::Io { .. } => ExitCode::SystemError,
            VendorError::SourceMissing { .. } => ExitCode::UserError,
            VendorError::Manifest(e) if e.is_system() => ExitCode::SystemError,
            VendorError::Lock(e) if e.is_system() => ExitCode::SystemError,
            VendorError::Drift { .. }
            | VendorError::MissingDependency { .. }
            | VendorError::Manifest(_)
            | VendorError::Lock(_)
            | VendorError::AuthConfig(_) => ExitCode::UserError,
            #[cfg(feature = "oci-fetch")]
            VendorError::OciFetch { .. } => ExitCode::UserError,
            #[cfg(feature = "git-fetch")]
            VendorError::GitFetch { .. } => ExitCode::UserError,
        }
    }
}

pub fn add(workspace: &Path, name: &str) -> Result<VendorAddOutput, VendorError> {
    let manifest = AkuaManifest::load(workspace)?;
    let dep = manifest
        .dependencies
        .get(name)
        .ok_or_else(|| VendorError::MissingDependency {
            name: name.to_string(),
        })?;
    let source = resolve_source(workspace, name, dep)?;
    let vendor_root = vendor_root(workspace);
    let target = vendor_root.join(name);
    let SyncOutcome { wrote, replaced } = sync_tree(&source.root, &target)?;
    let (digest, size_bytes) = hash_tree(&target).map_err(|err| VendorError::Io {
        path: target.clone(),
        source: err,
    })?;

    Ok(VendorAddOutput {
        name: name.to_string(),
        source_kind: source.kind.to_string(),
        source_ref: source.source_ref,
        path: target,
        digest,
        size_bytes,
        wrote,
        replaced,
    })
}

pub fn list(workspace: &Path) -> Result<VendorListOutput, VendorError> {
    let expected = expected_names(workspace)?;
    let vendor_root = vendor_root(workspace);
    let entries = scan_vendor_entries(&vendor_root, &expected)?;
    Ok(VendorListOutput { entries })
}

pub fn check(workspace: &Path) -> Result<VendorCheckOutput, VendorError> {
    let expected = expected_entries(workspace)?;
    let vendor_root = vendor_root(workspace);
    let on_disk = scan_vendor_entries(&vendor_root, &expected.names)?;

    let mut entries = Vec::new();
    let mut drift = false;
    let mut orphaned = Vec::new();
    let mut missing = Vec::new();

    let disk_map: BTreeMap<String, VendorListEntry> = on_disk
        .clone()
        .into_iter()
        .map(|entry| (entry.name.clone(), entry))
        .collect();

    for expected in expected.entries {
        let path = vendor_root.join(&expected.name);
        match disk_map.get(&expected.name) {
            Some(actual) => {
                let mismatch = actual.digest != expected.digest;
                drift |= mismatch;
                entries.push(VendorCheckEntry {
                    name: expected.name.clone(),
                    path,
                    source_kind: Some(expected.source_kind.to_string()),
                    source_ref: Some(expected.source_ref),
                    expected_digest: Some(expected.digest),
                    actual_digest: Some(actual.digest.clone()),
                    orphan: false,
                    missing_vendor: false,
                    missing_source: false,
                });
            }
            None => {
                drift = true;
                missing.push(expected.name.clone());
                entries.push(VendorCheckEntry {
                    name: expected.name.clone(),
                    path,
                    source_kind: Some(expected.source_kind.to_string()),
                    source_ref: Some(expected.source_ref),
                    expected_digest: Some(expected.digest),
                    actual_digest: None,
                    orphan: false,
                    missing_vendor: true,
                    missing_source: false,
                });
            }
        }
    }

    for entry in on_disk {
        if expected.names.contains(&entry.name) {
            continue;
        }
        drift = true;
        orphaned.push(entry.name.clone());
        entries.push(VendorCheckEntry {
            name: entry.name.clone(),
            path: entry.path,
            source_kind: None,
            source_ref: None,
            expected_digest: None,
            actual_digest: Some(entry.digest),
            orphan: true,
            missing_vendor: false,
            missing_source: false,
        });
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    orphaned.sort();
    missing.sort();

    let output = VendorCheckOutput {
        drift,
        entries,
        orphaned,
        missing,
    };

    if output.drift {
        Err(VendorError::Drift { output })
    } else {
        Ok(output)
    }
}

#[derive(Debug, Clone)]
struct ExpectedDep {
    name: String,
    source_kind: &'static str,
    source_ref: String,
    digest: String,
}

#[derive(Debug, Clone)]
struct ExpectedSet {
    names: std::collections::BTreeSet<String>,
    entries: Vec<ExpectedDep>,
}

#[derive(Debug, Clone)]
struct SourceResolution {
    kind: &'static str,
    source_ref: String,
    root: PathBuf,
}

#[derive(Debug, Clone, Copy)]
struct SyncOutcome {
    wrote: bool,
    replaced: bool,
}

fn expected_names(workspace: &Path) -> Result<std::collections::BTreeSet<String>, VendorError> {
    Ok(expected_entries(workspace)?.names)
}

fn expected_entries(workspace: &Path) -> Result<ExpectedSet, VendorError> {
    let manifest = AkuaManifest::load(workspace)?;
    let lock = match AkuaLock::load(workspace) {
        Ok(lock) => Some(lock),
        Err(LockLoadError::Missing { .. }) => None,
        Err(e) => return Err(VendorError::Lock(e)),
    };

    let mut names = std::collections::BTreeSet::new();
    let mut entries = Vec::new();
    for (name, dep) in &manifest.dependencies {
        names.insert(name.clone());
        if let Some(locked) = lock.as_ref().and_then(|l| l.find(name)) {
            let (source_kind, source_ref) = locked_source_meta(locked);
            entries.push(ExpectedDep {
                name: name.clone(),
                source_kind,
                source_ref,
                digest: locked.digest.clone(),
            });
            continue;
        }

        let source = resolve_source(workspace, name, dep)?;
        let (digest, _) = hash_tree(&source.root).map_err(|err| VendorError::Io {
            path: source.root.clone(),
            source: err,
        })?;
        entries.push(ExpectedDep {
            name: name.clone(),
            source_kind: source.kind,
            source_ref: source.source_ref,
            digest,
        });
    }

    if let Some(lock) = lock {
        for pkg in lock.packages {
            names.insert(pkg.name.clone());
            if manifest.dependencies.contains_key(&pkg.name) {
                continue;
            }
            let (source_kind, source_ref) = locked_source_meta(&pkg);
            entries.push(ExpectedDep {
                name: pkg.name,
                source_kind,
                source_ref,
                digest: pkg.digest,
            });
        }
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ExpectedSet { names, entries })
}

fn locked_source_meta(pkg: &LockedPackage) -> (&'static str, String) {
    if pkg.is_path() {
        ("path", pkg.source.clone())
    } else if pkg.is_oci() {
        ("oci", pkg.source.clone())
    } else {
        ("git", pkg.source.clone())
    }
}

fn resolve_source(
    workspace: &Path,
    name: &str,
    dep: &Dependency,
) -> Result<SourceResolution, VendorError> {
    let source_kind = dep
        .source()
        .expect("manifest validation guarantees exactly one source");
    match source_kind {
        DependencySource::Path => {
            let rel = dep.path.as_ref().expect("path dep has a path");
            let root = workspace.join(rel);
            if !root.is_dir() {
                return Err(VendorError::SourceMissing { path: root });
            }
            Ok(SourceResolution {
                kind: "path",
                source_ref: rel.clone(),
                root,
            })
        }
        DependencySource::Oci => resolve_oci_source(workspace, name, dep),
        DependencySource::Git => resolve_git_source(workspace, name, dep),
    }
}

#[cfg(feature = "oci-fetch")]
fn resolve_oci_source(
    workspace: &Path,
    name: &str,
    dep: &Dependency,
) -> Result<SourceResolution, VendorError> {
    let oci = dep.oci.as_ref().expect("oci dep has oci ref");
    let version = dep.version.as_deref().expect("oci dep has version");
    let cache_root = cache_inventory::default_cache_root("oci");
    let creds = oci_auth::CredsStore::load().map_err(|e| VendorError::AuthConfig(e.to_string()))?;
    let locked_expected = expected_digest_from_lock(workspace, name, "oci");
    let fetch_opts = oci_fetcher::FetchOpts {
        expected_digest: locked_expected.as_deref(),
        creds: &creds,
        cosign_public_key_pem: None,
    };
    let fetched =
        oci_fetcher::fetch_with_opts(oci, version, &cache_root, &fetch_opts).map_err(|source| {
            VendorError::OciFetch {
                name: name.to_string(),
                source,
            }
        })?;
    Ok(SourceResolution {
        kind: "oci",
        source_ref: format!("{oci}@{version}"),
        root: fetched.root_dir,
    })
}

#[cfg(not(feature = "oci-fetch"))]
fn resolve_oci_source(
    _workspace: &Path,
    name: &str,
    _dep: &Dependency,
) -> Result<SourceResolution, VendorError> {
    let _ = name;
    Err(VendorError::SourceMissing {
        path: PathBuf::from("oci-fetch feature disabled"),
    })
}

#[cfg(feature = "git-fetch")]
fn resolve_git_source(
    workspace: &Path,
    name: &str,
    dep: &Dependency,
) -> Result<SourceResolution, VendorError> {
    let git = dep.git.as_ref().expect("git dep has git ref");
    let ref_spec = match (dep.tag.as_ref(), dep.rev.as_ref()) {
        (Some(tag), _) => RefSpec::Tag(tag.clone()),
        (_, Some(rev)) => RefSpec::Rev(rev.clone()),
        _ => unreachable!("manifest validation guarantees tag or rev"),
    };
    let cache_root = cache_inventory::default_cache_root("git");
    let expected_commit = expected_digest_from_lock(workspace, name, "git")
        .and_then(|d| d.strip_prefix("git:").map(str::to_string));
    let fetched = git_fetcher::fetch(git, &ref_spec, &cache_root, expected_commit.as_deref())
        .map_err(|source| VendorError::GitFetch {
            name: name.to_string(),
            source,
        })?;
    Ok(SourceResolution {
        kind: "git",
        source_ref: format!("{git}@{}", ref_spec.label()),
        root: fetched.chart_dir,
    })
}

#[cfg(not(feature = "git-fetch"))]
fn resolve_git_source(
    _workspace: &Path,
    name: &str,
    _dep: &Dependency,
) -> Result<SourceResolution, VendorError> {
    let _ = name;
    Err(VendorError::SourceMissing {
        path: PathBuf::from("git-fetch feature disabled"),
    })
}

fn expected_digest_from_lock(workspace: &Path, name: &str, kind: &str) -> Option<String> {
    let lock = AkuaLock::load(workspace).ok()?;
    let pkg = lock.find(name)?;
    match (
        kind,
        pkg.is_oci(),
        pkg.is_path(),
        pkg.source.starts_with("git+"),
    ) {
        ("oci", true, _, _) => Some(pkg.digest.clone()),
        ("git", _, _, true) => Some(pkg.digest.clone()),
        _ => None,
    }
}

fn vendor_root(workspace: &Path) -> PathBuf {
    workspace.join(".akua/vendor")
}

fn scan_vendor_entries(
    vendor_root: &Path,
    expected: &std::collections::BTreeSet<String>,
) -> Result<Vec<VendorListEntry>, VendorError> {
    let mut entries = Vec::new();
    let Ok(rd) = std::fs::read_dir(vendor_root) else {
        return Ok(entries);
    };
    for entry in rd {
        let entry = entry.map_err(|source| VendorError::Io {
            path: vendor_root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let (digest, size_bytes) = hash_tree(&path).map_err(|source| VendorError::Io {
            path: path.clone(),
            source,
        })?;
        entries.push(VendorListEntry {
            name: name.clone(),
            path,
            digest,
            size_bytes,
            orphan: !expected.contains(&name),
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

fn sync_tree(source: &Path, target: &Path) -> Result<SyncOutcome, VendorError> {
    let (source_digest, _) = hash_tree(source).map_err(|err| VendorError::Io {
        path: source.to_path_buf(),
        source: err,
    })?;
    if target.exists() {
        let (target_digest, _) = hash_tree(target).map_err(|err| VendorError::Io {
            path: target.to_path_buf(),
            source: err,
        })?;
        if target_digest == source_digest {
            return Ok(SyncOutcome {
                wrote: false,
                replaced: false,
            });
        }
        std::fs::remove_dir_all(target).map_err(|err| VendorError::Io {
            path: target.to_path_buf(),
            source: err,
        })?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|err| VendorError::Io {
                path: parent.to_path_buf(),
                source: err,
            })?;
        }
        copy_tree(source, target)?;
        return Ok(SyncOutcome {
            wrote: true,
            replaced: true,
        });
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|err| VendorError::Io {
            path: parent.to_path_buf(),
            source: err,
        })?;
    }
    copy_tree(source, target)?;
    Ok(SyncOutcome {
        wrote: true,
        replaced: false,
    })
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), VendorError> {
    std::fs::create_dir_all(target).map_err(|err| VendorError::Io {
        path: target.to_path_buf(),
        source: err,
    })?;
    let rd = std::fs::read_dir(source).map_err(|err| VendorError::Io {
        path: source.to_path_buf(),
        source: err,
    })?;
    for entry in rd {
        let entry = entry.map_err(|err| VendorError::Io {
            path: target.to_path_buf(),
            source: err,
        })?;
        let ft = entry.file_type().map_err(|err| VendorError::Io {
            path: entry.path(),
            source: err,
        })?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        if ft.is_dir() {
            copy_tree(&from, &to)?;
        } else if ft.is_file() {
            std::fs::copy(&from, &to).map_err(|err| VendorError::Io {
                path: from,
                source: err,
            })?;
        }
    }
    Ok(())
}

fn hash_tree(root: &Path) -> Result<(String, u64), std::io::Error> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Sha256::new();
    let mut size_bytes = 0_u64;
    for (rel, abs) in files {
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update(b"\0");
        let file = std::fs::File::open(&abs)?;
        size_bytes = size_bytes.saturating_add(file.metadata()?.len());
        let mut reader = std::io::BufReader::new(file);
        std::io::copy(&mut reader, &mut hasher)?;
        hasher.update(b"\n");
    }
    Ok((
        format!("sha256:{}", hex::encode(hasher.finalize())),
        size_bytes,
    ))
}

fn collect_files(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<(), std::io::Error> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let path = entry.path();
        if ft.is_dir() {
            collect_files(root, &path, out)?;
        } else if ft.is_file() {
            let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            out.push((rel, path));
        }
    }
    Ok(())
}

mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        let bytes = bytes.as_ref();
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push(hex_char(b >> 4));
            out.push(hex_char(b & 0x0f));
        }
        out
    }

    fn hex_char(nibble: u8) -> char {
        match nibble {
            0..=9 => (b'0' + nibble) as char,
            10..=15 => (b'a' + nibble - 10) as char,
            _ => unreachable!("nibble out of range"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(root: &Path, rel: &str, body: &[u8]) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn workspace(manifest: &str) -> TempDir {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("akua.toml"), manifest).unwrap();
        tmp
    }

    fn minimal_manifest() -> &'static str {
        r#"
[package]
name = "vendor-test"
version = "0.1.0"
edition = "akua.dev/v1alpha1"

[dependencies]
local = { path = "./charts/local" }
"#
    }

    fn make_source_tree(root: &Path) {
        write(
            root,
            "Chart.yaml",
            b"apiVersion: v2\nname: local\nversion: 0.1.0\n",
        );
        write(root, "templates/a.yaml", b"a: 1\n");
        write(root, "templates/b.yaml", b"b: 2\n");
    }

    #[test]
    fn add_copies_declared_path_dep_into_vendor_tree() {
        let ws = workspace(minimal_manifest());
        make_source_tree(&ws.path().join("charts/local"));

        let out = add(ws.path(), "local").expect("add");
        assert!(out.wrote);
        assert_eq!(out.source_kind, "path");
        assert!(out.path.ends_with(".akua/vendor/local"));
        assert!(out.path.join("Chart.yaml").is_file());
        assert!(out.digest.starts_with("sha256:"));

        let copied = std::fs::read_to_string(out.path.join("templates/a.yaml")).unwrap();
        assert!(copied.contains("a: 1"));
    }

    #[test]
    fn add_is_idempotent_when_vendor_tree_already_matches_source() {
        let ws = workspace(minimal_manifest());
        make_source_tree(&ws.path().join("charts/local"));

        let first = add(ws.path(), "local").expect("first add");
        let second = add(ws.path(), "local").expect("second add");
        assert!(first.wrote);
        assert!(!second.wrote);
        assert_eq!(first.digest, second.digest);
    }

    #[test]
    fn check_reports_drift_after_vendored_tree_changes() {
        let ws = workspace(minimal_manifest());
        make_source_tree(&ws.path().join("charts/local"));
        add(ws.path(), "local").expect("add");

        std::fs::write(
            ws.path().join(".akua/vendor/local/templates/a.yaml"),
            b"a: 99\n",
        )
        .unwrap();

        let err = check(ws.path()).expect_err("drift");
        let structured = err.to_structured();
        assert_eq!(structured.code, codes::E_VENDOR_DRIFT);
        assert_eq!(err.exit_code(), ExitCode::UserError);
        match err {
            VendorError::Drift { output } => {
                assert!(output.drift);
                assert!(output
                    .entries
                    .iter()
                    .any(|e| e.name == "local" && e.actual_digest.is_some()));
            }
            other => panic!("expected drift, got {other:?}"),
        }
    }

    #[test]
    fn list_marks_orphaned_vendor_trees() {
        let ws = workspace(minimal_manifest());
        make_source_tree(&ws.path().join("charts/local"));
        add(ws.path(), "local").expect("add");

        write(
            ws.path(),
            ".akua/vendor/orphan/Chart.yaml",
            b"name: orphan\n",
        );

        let listed = list(ws.path()).expect("list");
        assert_eq!(listed.entries.len(), 2);
        let orphan = listed
            .entries
            .iter()
            .find(|e| e.name == "orphan")
            .expect("orphan entry");
        assert!(orphan.orphan);
    }

    #[test]
    fn missing_dependency_suggests_vendor_path_declaration() {
        let ws = workspace(minimal_manifest());
        let err = add(ws.path(), "nope").unwrap_err();
        let structured = err.to_structured();
        assert_eq!(structured.code, codes::E_VENDOR_DEP_MISSING);
        assert!(structured
            .suggestion
            .unwrap_or_default()
            .contains("path = \".akua/vendor/<name>\""));
    }
}
