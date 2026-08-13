//! `akuapkg vendor` — materialize, inspect, and drift-check the workspace
//! vendor tree at `.akua/vendor/<name>/`.
//!
//! Subcommands:
//! - `akuapkg vendor add <name>` — copy the declared dep into the vendor tree.
//! - `akuapkg vendor check` — compare the vendor tree against the manifest/lock.
//! - `akuapkg vendor list` — inventory the on-disk vendor trees, including orphans.
//!
//! This module also keeps the shared `collect_vendor_pairs` helper used by
//! `akuapkg pack` and `akuapkg publish`. The helper lives here because it emits a
//! stderr warning on resolver failure, which is CLI-layer behavior.

use std::io::Write;
use std::path::{Path, PathBuf};

use akua_core::cli_contract::{codes, ExitCode, StructuredError};
use akua_core::vendor as core_vendor;
use akua_core::AkuaManifest;

use crate::contract::{emit_output, Context};

pub use core_vendor::{
    VendorAddOutput, VendorCheckEntry, VendorCheckOutput, VendorError, VendorListEntry,
    VendorListOutput,
};

#[derive(Debug, Clone)]
pub enum VendorAction<'a> {
    Add { workspace: &'a Path, name: &'a str },
    Check { workspace: &'a Path },
    List { workspace: &'a Path },
}

#[derive(Debug, Clone)]
pub struct VendorArgs<'a> {
    pub action: VendorAction<'a>,
    /// Credentials for private remotes. Only consulted by the `Add`
    /// action; `Check` / `List` operate on the lockfile + on-disk
    /// vendor tree and never touch the network.
    pub auth: Option<akua_core::host_auth::HostAuthMap>,
}

#[derive(Debug, thiserror::Error)]
pub enum VendorVerbError {
    #[error(transparent)]
    Vendor(#[from] VendorError),

    #[error("write to stdout failed: {0}")]
    StdoutWrite(#[source] std::io::Error),
}

impl VendorVerbError {
    pub fn to_structured(&self) -> StructuredError {
        match self {
            VendorVerbError::Vendor(err) => err.to_structured(),
            VendorVerbError::StdoutWrite(err) => {
                StructuredError::new(codes::E_IO, err.to_string()).with_default_docs()
            }
        }
    }

    pub fn exit_code(&self) -> ExitCode {
        match self {
            VendorVerbError::Vendor(err) => err.exit_code(),
            VendorVerbError::StdoutWrite(_) => ExitCode::SystemError,
        }
    }
}

pub fn run<W: Write>(
    ctx: &Context,
    args: &VendorArgs<'_>,
    stdout: &mut W,
) -> Result<ExitCode, VendorVerbError> {
    match &args.action {
        VendorAction::Add { workspace, name } => {
            let output = if ctx.plan {
                core_vendor::plan_add(workspace, name, args.auth.as_ref())?
            } else {
                core_vendor::add(workspace, name, args.auth.as_ref())?
            };
            emit_output(stdout, ctx, &output, |w| {
                write_add_text(w, &output, ctx.plan)
            })
            .map_err(VendorVerbError::StdoutWrite)?;
            Ok(ExitCode::Success)
        }
        VendorAction::Check { workspace } => match core_vendor::check(workspace) {
            Ok(output) => {
                emit_output(stdout, ctx, &output, |w| write_check_text(w, &output))
                    .map_err(VendorVerbError::StdoutWrite)?;
                Ok(ExitCode::Success)
            }
            Err(VendorError::Drift { output }) => {
                emit_output(stdout, ctx, &output, |w| write_check_text(w, &output))
                    .map_err(VendorVerbError::StdoutWrite)?;
                Err(VendorVerbError::Vendor(VendorError::Drift { output }))
            }
            Err(err) => Err(VendorVerbError::Vendor(err)),
        },
        VendorAction::List { workspace } => {
            let output = core_vendor::list(workspace)?;
            emit_output(stdout, ctx, &output, |w| write_list_text(w, &output))
                .map_err(VendorVerbError::StdoutWrite)?;
            Ok(ExitCode::Success)
        }
    }
}

/// Resolve non-path deps so their chart content can be vendored into the
/// output tarball. Path deps already live in the workspace tree (typically
/// `vendor/`) and are packed via the workspace walk — don't re-vendor them or
/// they'll appear twice in the tarball.
///
/// A resolver failure here is loud: we emit a stderr warning so the publisher
/// doesn't ship an un-vendored artifact by accident. Returns the pairs the
/// resolver did produce — a partial-vendor result is still better than nothing
/// when one dep out of many is broken.
pub fn collect_vendor_pairs(workspace: &Path, manifest: &AkuaManifest) -> Vec<(String, PathBuf)> {
    use akua_core::chart_resolver::{self, ResolvedSource, ResolverOptions};
    use akua_core::AkuaLock;

    let expected_digests = AkuaLock::load(workspace)
        .map(|lock| {
            lock.packages
                .into_iter()
                .filter(|p| p.is_oci())
                .map(|p| (p.name, p.digest))
                .collect()
        })
        .unwrap_or_default();
    let opts = ResolverOptions {
        offline: false,
        cache_root: None,
        expected_digests,
        cosign_public_key_pem: None,
        reject_replace: chart_resolver::replace_rejected_from_env(),
        auth: None,
    };
    let resolved = match chart_resolver::resolve_with_options(manifest, workspace, &opts) {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "warning: dep resolution failed, packed artifact will not render offline: {e}"
            );
            return Vec::new();
        }
    };

    let mut pairs = Vec::new();
    for chart in resolved.entries.values() {
        // Path / replace -> already in the workspace walk; don't double-vendor.
        let include = matches!(
            chart.source,
            ResolvedSource::Oci { .. } | ResolvedSource::Git { .. }
        );
        if include {
            pairs.push((chart.name.clone(), chart.abs_path.clone()));
        }
    }
    pairs
}

fn write_add_text<W: Write>(
    w: &mut W,
    out: &VendorAddOutput,
    planned: bool,
) -> std::io::Result<()> {
    if planned {
        writeln!(w, "plan: vendor add {}", out.name)?;
    }
    writeln!(w, "vendor {}", out.name)?;
    writeln!(w, "  source  {} {}", out.source_kind, out.source_ref)?;
    writeln!(w, "  path    {}", out.path.display())?;
    writeln!(w, "  digest  {}", out.digest)?;
    writeln!(w, "  size    {} bytes", out.size_bytes)?;
    writeln!(w, "  wrote   {}", out.wrote)?;
    writeln!(w, "  replaced {}", out.replaced)?;
    Ok(())
}

fn write_list_text<W: Write>(w: &mut W, out: &VendorListOutput) -> std::io::Result<()> {
    if out.entries.is_empty() {
        writeln!(w, "no vendored trees")?;
        return Ok(());
    }
    for entry in &out.entries {
        let orphan = if entry.orphan { " (orphan)" } else { "" };
        writeln!(
            w,
            "  {}{}  {}  {} bytes  {}",
            entry.name,
            orphan,
            entry.digest,
            entry.size_bytes,
            entry.path.display()
        )?;
    }
    Ok(())
}

fn write_check_text<W: Write>(w: &mut W, out: &VendorCheckOutput) -> std::io::Result<()> {
    for entry in &out.entries {
        let marker = if entry.orphan {
            "(orphan)"
        } else if entry.missing_vendor {
            "(missing vendor)"
        } else if entry.missing_source {
            "(missing source)"
        } else if entry.expected_digest != entry.actual_digest {
            "(drift)"
        } else {
            "(ok)"
        };
        writeln!(w, "  {} {}", entry.name, marker)?;
        if let Some(kind) = &entry.source_kind {
            if let Some(source_ref) = &entry.source_ref {
                writeln!(w, "    source  {} {}", kind, source_ref)?;
            } else {
                writeln!(w, "    source  {}", kind)?;
            }
        }
        if let Some(expected) = &entry.expected_digest {
            writeln!(w, "    expected {}", expected)?;
        }
        if let Some(actual) = &entry.actual_digest {
            writeln!(w, "    actual   {}", actual)?;
        }
    }
    if !out.orphaned.is_empty() {
        writeln!(w, "  orphaned: {}", out.orphaned.join(", "))?;
    }
    if !out.missing.is_empty() {
        writeln!(w, "  missing: {}", out.missing.join(", "))?;
    }
    writeln!(w, "{}", if out.drift { "drift" } else { "ok" })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::workspace_with;
    use std::fs;

    const NO_DEPS: &str = r#"[package]
name = "vendor-test"
version = "0.0.1"
edition = "akua.dev/v1alpha1"
"#;

    #[test]
    fn empty_manifest_yields_empty_vendor_pairs() {
        let ws = workspace_with(NO_DEPS);
        let manifest = AkuaManifest::load(ws.path()).unwrap();
        let pairs = collect_vendor_pairs(ws.path(), &manifest);
        assert!(pairs.is_empty(), "no deps -> no vendor pairs");
    }

    #[test]
    fn path_dep_is_excluded_from_vendor_pairs() {
        let ws = workspace_with(&format!(
            "{NO_DEPS}\n[dependencies]\nlocal = {{ path = \"./local-chart\" }}\n"
        ));
        let chart_dir = ws.path().join("local-chart");
        fs::create_dir(&chart_dir).unwrap();
        fs::create_dir(chart_dir.join("templates")).unwrap();
        fs::write(
            chart_dir.join("Chart.yaml"),
            "apiVersion: v2\nname: local\nversion: 0.0.1\n",
        )
        .unwrap();
        fs::write(chart_dir.join("templates/cm.yaml"), "kind: ConfigMap\n").unwrap();

        let manifest = AkuaManifest::load(ws.path()).unwrap();
        let pairs = collect_vendor_pairs(ws.path(), &manifest);
        assert!(pairs.is_empty(), "path dep must NOT appear in vendor pairs");
    }

    #[test]
    fn resolver_failure_returns_empty_vec_after_warning() {
        let ws = workspace_with(&format!(
            "{NO_DEPS}\n[dependencies]\nbroken = {{ path = \"./does-not-exist\" }}\n"
        ));
        let manifest = AkuaManifest::load(ws.path()).unwrap();
        let pairs = collect_vendor_pairs(ws.path(), &manifest);
        assert!(pairs.is_empty(), "resolver-failure path returns empty Vec");
    }
}
