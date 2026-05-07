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

use crate::chart_resolver::{hash_dir, validate_workspace_path, WorkspacePathError, VENDOR_DIR};
use crate::cli_contract::{codes, ExitCode, StructuredError};
use crate::lock_file::{AkuaLock, LockLoadError, LockedPackage, GIT_DIGEST_PREFIX};
use crate::mod_file::{AkuaManifest, Dependency, DependencySource, ManifestLoadError};
#[cfg(feature = "oci-fetch")]
use crate::{cache_inventory, oci_auth, oci_fetcher};
#[cfg(feature = "git-fetch")]
use crate::{git_fetcher, git_fetcher::RefSpec};
use serde::{Deserialize, Serialize};

crate::contract_type! {
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct VendorListEntry {
        pub name: String,
        pub path: PathBuf,
        pub digest: String,
        pub size_bytes: u64,
        pub orphan: bool,
    }
}

crate::contract_type! {
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct VendorListOutput {
        pub entries: Vec<VendorListEntry>,
    }
}

crate::contract_type! {
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
}

crate::contract_type! {
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct VendorCheckOutput {
        pub drift: bool,
        pub entries: Vec<VendorCheckEntry>,
        pub orphaned: Vec<String>,
        pub missing: Vec<String>,
    }
}

crate::contract_type! {
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

    /// Absolute path in `akua.toml` `path = "..."`. Path deps must
    /// stay under the workspace root — see CLAUDE.md "`replace` and
    /// `path` deps are workspace-local; never cross Package
    /// boundaries". Same invariant `chart_resolver` enforces.
    #[error(
        "dep `{name}`: absolute path `{}` is rejected — \
         path deps must stay under the workspace root",
        path.display()
    )]
    AbsolutePathRejected { name: String, path: PathBuf },

    /// Relative path that canonicalizes outside the workspace
    /// (typically via `..` segments or symlinks). Same workspace-local
    /// invariant as [`Self::AbsolutePathRejected`].
    #[error(
        "dep `{name}`: path `{}` resolves to `{}`, which escapes \
         the workspace root `{}`",
        requested.display(), resolved.display(), workspace_root.display()
    )]
    PathEscape {
        name: String,
        requested: PathBuf,
        resolved: PathBuf,
        workspace_root: PathBuf,
    },

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
            VendorError::MissingDependency { name } => {
                StructuredError::new(codes::E_VENDOR_DEP_MISSING, self.to_string())
                    .with_suggestion(format!(
                        "declare `path = \".akua/vendor/{name}\"` in `akua.toml` first"
                    ))
                    .with_default_docs()
            }
            VendorError::SourceMissing { path } => {
                StructuredError::new(codes::E_IO, self.to_string())
                    .with_path(path.display().to_string())
                    .with_default_docs()
            }
            VendorError::AbsolutePathRejected { path, .. } => {
                StructuredError::new(codes::E_PATH_ESCAPE, self.to_string())
                    .with_path(path.display().to_string())
                    .with_suggestion(
                        "use a relative path under the workspace, or vendor the \
                         dep via `oci = \"...\"` / `git = \"...\"`",
                    )
                    .with_default_docs()
            }
            VendorError::PathEscape { resolved, .. } => {
                StructuredError::new(codes::E_PATH_ESCAPE, self.to_string())
                    .with_path(resolved.display().to_string())
                    .with_suggestion(
                        "rewrite the path to stay under the workspace, or move \
                         the source into the workspace",
                    )
                    .with_default_docs()
            }
            VendorError::Io { path, source } => {
                StructuredError::new(codes::E_IO, source.to_string())
                    .with_path(path.display().to_string())
                    .with_default_docs()
            }
            #[cfg(feature = "oci-fetch")]
            VendorError::OciFetch { .. } => {
                StructuredError::new(codes::E_DEP_RESOLVE, self.to_string()).with_default_docs()
            }
            #[cfg(feature = "git-fetch")]
            VendorError::GitFetch { .. } => {
                StructuredError::new(codes::E_DEP_RESOLVE, self.to_string()).with_default_docs()
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
            | VendorError::AbsolutePathRejected { .. }
            | VendorError::PathEscape { .. }
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
    add_impl(workspace, name, true)
}

pub fn plan_add(workspace: &Path, name: &str) -> Result<VendorAddOutput, VendorError> {
    add_impl(workspace, name, false)
}

fn add_impl(workspace: &Path, name: &str, write: bool) -> Result<VendorAddOutput, VendorError> {
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

    // Source digest is also the post-copy target digest — `copy_tree` is
    // byte-identical, so we don't re-hash the target after writing.
    let (digest, size_bytes) = hash_dir(&source.root).map_err(|err| VendorError::Io {
        path: source.root.clone(),
        source: err,
    })?;

    let target_existed = target.exists();
    let target_matches = if target_existed {
        let (target_digest, _) = hash_dir(&target).map_err(|err| VendorError::Io {
            path: target.clone(),
            source: err,
        })?;
        target_digest == digest
    } else {
        false
    };

    let (wrote, replaced) = if target_matches {
        (false, false)
    } else if write {
        if target_existed {
            std::fs::remove_dir_all(&target).map_err(|err| VendorError::Io {
                path: target.clone(),
                source: err,
            })?;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|err| VendorError::Io {
                path: parent.to_path_buf(),
                source: err,
            })?;
        }
        copy_tree(&source.root, &target)?;
        (true, target_existed)
    } else {
        // plan mode: report what would happen without writing
        (true, target_existed)
    };

    // Pin the vendored tree in `akua.lock` (write mode only — plan mode
    // is read-only by contract). Render's vendor-first lookup hashes the
    // tree at resolve time, but the lockfile is what `vendor check`
    // compares against to detect drift; without this pin a vendored dep
    // would always re-hash the source (or fail if the source was GC'd).
    if write {
        upsert_vendor_lock_entry(workspace, name, dep, &digest)?;
    }

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

/// Upsert a `LockedPackage` for the vendored dep. Mirrors what
/// `chart_resolver::merge_into_lock` writes for the same dep when
/// resolved via vendor-first — same `source` / `version` / `digest`
/// shape so `vendor check` and `akua verify` produce consistent
/// results regardless of which path produced the lockfile.
fn upsert_vendor_lock_entry(
    workspace: &Path,
    name: &str,
    dep: &Dependency,
    tree_digest: &str,
) -> Result<(), VendorError> {
    let mut lock = match AkuaLock::load(workspace) {
        Ok(l) => l,
        Err(LockLoadError::Missing { .. }) => AkuaLock::empty(),
        Err(e) => return Err(VendorError::Lock(e)),
    };
    let prior = lock.find(name).cloned();
    let kind = dep
        .source()
        .expect("manifest validation guarantees exactly one source");
    let (source, version, digest) = match kind {
        DependencySource::Path => {
            let declared = dep.path.as_ref().expect("path dep has path").clone();
            (
                format!("path+file://{declared}"),
                "local".to_string(),
                tree_digest.to_string(),
            )
        }
        DependencySource::Oci => {
            let oci = dep.oci.as_ref().expect("oci dep has oci ref").clone();
            let version = dep
                .version
                .clone()
                .unwrap_or_else(|| "vendored".to_string());
            (oci, version, tree_digest.to_string())
        }
        DependencySource::Git => {
            let git = dep.git.as_ref().expect("git dep has git ref").clone();
            let tag_or_rev = dep
                .tag
                .clone()
                .or_else(|| dep.rev.clone())
                .unwrap_or_else(|| "vendored".to_string());
            // Git lockfile digest is `git:<tree-hex>` per the resolver's
            // existing convention (see `chart_resolver::resolve_from_vendor`
            // and `vendored_git_dep_resolves_locally` test).
            let raw = tree_digest
                .strip_prefix("sha256:")
                .unwrap_or(tree_digest)
                .to_string();
            (
                format!("git+{git}@{tag_or_rev}"),
                tag_or_rev,
                format!("{GIT_DIGEST_PREFIX}{raw}"),
            )
        }
    };
    lock.upsert(LockedPackage {
        name: name.to_string(),
        version,
        source,
        digest,
        // Cosign / SLSA / replace metadata is owned by `akua publish` and
        // resolver-side replace handling; preserve whatever the prior
        // entry recorded so re-vendoring doesn't lose them.
        signature: prior.as_ref().and_then(|p| p.signature.clone()),
        attestation: prior.as_ref().and_then(|p| p.attestation.clone()),
        dependencies: prior
            .as_ref()
            .map(|p| p.dependencies.clone())
            .unwrap_or_default(),
        replaced: prior.as_ref().and_then(|p| p.replaced.clone()),
        yanked: prior.as_ref().and_then(|p| p.yanked),
        kyverno_source_digest: prior.as_ref().and_then(|p| p.kyverno_source_digest.clone()),
        converter_version: prior.as_ref().and_then(|p| p.converter_version.clone()),
    });
    lock.save(workspace).map_err(VendorError::Lock)?;
    Ok(())
}

pub fn list(workspace: &Path) -> Result<VendorListOutput, VendorError> {
    let expected = expected_entries(workspace)?.names;
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

    let disk_map: BTreeMap<&str, &VendorListEntry> = on_disk
        .iter()
        .map(|entry| (entry.name.as_str(), entry))
        .collect();

    for expected in expected.entries {
        let path = vendor_root.join(&expected.name);
        match disk_map.get(expected.name.as_str()) {
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

    drop(disk_map);
    for entry in on_disk {
        if expected.names.contains(&entry.name) {
            continue;
        }
        drift = true;
        orphaned.push(entry.name.clone());
        entries.push(VendorCheckEntry {
            name: entry.name,
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
        let (digest, _) = hash_dir(&source.root).map_err(|err| VendorError::Io {
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

/// Validate + canonicalize a `path = "..."` dep target. Path deps must
/// canonicalize under the workspace root; otherwise vendor would be a
/// side-channel for copying arbitrary host bytes (or anything the
/// calling user can read) into `.akua/vendor/<name>/` where the
/// install pipeline would commit them. Same workspace-local invariant
/// `chart_resolver::resolve_path` enforces, via the shared helper.
fn resolve_path_dep(workspace: &Path, name: &str, requested: &str) -> Result<PathBuf, VendorError> {
    let workspace_canon = workspace.canonicalize().map_err(|err| VendorError::Io {
        path: workspace.to_path_buf(),
        source: err,
    })?;
    let canon =
        validate_workspace_path(&workspace_canon, workspace, requested).map_err(
            |err| match err {
                WorkspacePathError::AbsoluteRejected { path } => {
                    VendorError::AbsolutePathRejected {
                        name: name.to_string(),
                        path,
                    }
                }
                WorkspacePathError::NotFound { path } => VendorError::SourceMissing { path },
                WorkspacePathError::Io { path, source } => VendorError::Io { path, source },
                WorkspacePathError::Escape {
                    resolved,
                    workspace_root,
                } => VendorError::PathEscape {
                    name: name.to_string(),
                    requested: PathBuf::from(requested),
                    resolved,
                    workspace_root,
                },
            },
        )?;
    // canonicalize already required existence; one extra stat to confirm
    // the target is a directory (vendor only mounts directories).
    if !canon.is_dir() {
        return Err(VendorError::SourceMissing { path: canon });
    }
    Ok(canon)
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
            let root = resolve_path_dep(workspace, name, rel)?;
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
    workspace.join(VENDOR_DIR)
}

fn scan_vendor_entries(
    vendor_root: &Path,
    expected: &std::collections::BTreeSet<String>,
) -> Result<Vec<VendorListEntry>, VendorError> {
    let mut entries = Vec::new();
    let rd = match std::fs::read_dir(vendor_root) {
        Ok(rd) => rd,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(entries),
        Err(err) => {
            return Err(VendorError::Io {
                path: vendor_root.to_path_buf(),
                source: err,
            });
        }
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
        let (digest, size_bytes) = hash_dir(&path).map_err(|source| VendorError::Io {
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
            .contains("path = \".akua/vendor/nope\""));
    }

    /// `vendor add` writes the dep's digest into `akua.lock` so render's
    /// vendor-first lookup + `vendor check` drift detection have a stable
    /// pin to compare against. Without this, dropping the canonical
    /// source after vendoring would break `vendor check`.
    #[test]
    fn add_pins_vendored_tree_in_akua_lock() {
        let ws = workspace(minimal_manifest());
        make_source_tree(&ws.path().join("charts/local"));

        let out = add(ws.path(), "local").expect("add");
        assert!(out.wrote);

        let lock_path = ws.path().join("akua.lock");
        assert!(lock_path.is_file(), "akua.lock should be written");

        let lock = AkuaLock::load(ws.path()).expect("load lock");
        let entry = lock.find("local").expect("local entry pinned");
        assert_eq!(entry.name, "local");
        assert_eq!(entry.version, "local");
        assert_eq!(entry.source, "path+file://./charts/local");
        assert_eq!(entry.digest, out.digest);
    }

    /// Re-running `vendor add` against an unchanged source produces a
    /// byte-identical lockfile — the lock-write must be idempotent.
    #[test]
    fn add_lockfile_write_is_idempotent_on_repeat() {
        let ws = workspace(minimal_manifest());
        make_source_tree(&ws.path().join("charts/local"));

        add(ws.path(), "local").expect("add 1");
        let lock_v1 = std::fs::read_to_string(ws.path().join("akua.lock")).expect("read 1");

        add(ws.path(), "local").expect("add 2");
        let lock_v2 = std::fs::read_to_string(ws.path().join("akua.lock")).expect("read 2");

        assert_eq!(lock_v1, lock_v2, "repeat add must produce identical lock");
    }

    /// Path deps must stay under the workspace root. Absolute paths
    /// in `path = "..."` are rejected with `E_PATH_ESCAPE` — same
    /// workspace-local invariant the chart_resolver enforces (see
    /// CLAUDE.md).
    #[test]
    fn absolute_path_dep_is_rejected_with_e_path_escape() {
        let ws = workspace(
            r#"
[package]
name = "vendor-test"
version = "0.1.0"
edition = "akua.dev/v1alpha1"

[dependencies]
bad = { path = "/etc" }
"#,
        );
        let err = add(ws.path(), "bad").unwrap_err();
        let structured = err.to_structured();
        assert_eq!(structured.code, codes::E_PATH_ESCAPE);
        assert!(matches!(err, VendorError::AbsolutePathRejected { .. }));
    }

    /// Relative paths that canonicalize outside the workspace
    /// (typically via `..` segments or symlinks) are rejected with
    /// `E_PATH_ESCAPE`, even though they look workspace-local in
    /// the manifest.
    #[test]
    fn path_dep_escaping_via_dotdot_is_rejected_with_e_path_escape() {
        let outer = tempfile::tempdir().expect("outer tmpdir");
        let workspace_dir = outer.path().join("ws");
        std::fs::create_dir(&workspace_dir).expect("create workspace");
        // The sibling just has to exist as a directory so canonicalize
        // resolves it. Rejection lands before any chart parsing.
        let sibling = outer.path().join("sibling");
        std::fs::create_dir(&sibling).expect("create sibling");
        std::fs::write(
            workspace_dir.join("akua.toml"),
            r#"
[package]
name = "escape_test"
version = "0.0.1"
edition = "akua.dev/v1alpha1"

[dependencies]
bad = { path = "../sibling" }
"#,
        )
        .expect("write manifest");

        let err = add(&workspace_dir, "bad").unwrap_err();
        let structured = err.to_structured();
        assert_eq!(structured.code, codes::E_PATH_ESCAPE);
        assert!(matches!(err, VendorError::PathEscape { .. }));
    }
}
