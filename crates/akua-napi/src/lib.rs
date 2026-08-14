//! `akua-napi` — Node.js native addon exposing `akua-core` via the
//! Node-API ABI. Loaded by `@akua-dev/sdk` per-platform; covers Node 22+,
//! Bun, and Deno (all three implement Node-API). The wasm32-unknown-
//! unknown bundle stays for browsers + pure-KCL fast path.
//!
//! Scope: thin pass-through bindings. Every function delegates to the
//! matching `akuapkg_cli::verbs::*::run` entry, capturing the `--json`
//! envelope to stdout and parsing it back into a `serde_json::Value`
//! for the JS caller. Zero envelope divergence from the CLI: same
//! bytes, different transport.

#![deny(clippy::all)]

use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;

use akua_core::cli_contract::{ExitCode, StructuredError};
use akua_core::oci_puller::OciPullError;
use akua_core::oci_transport::TransportError;
use akua_core::vendor as core_vendor;
use akuapkg_cli::contract::{emit_output, Context};
use akuapkg_cli::verbs;
use napi::bindgen_prelude::*;
use napi_derive::napi;

/// Routes through `verbs::version::run` so the JSON envelope stays
/// byte-stable with the CLI (`akua version --json`). Picking up
/// future fields the verb adds is automatic — no per-binding shape
/// drift like a `String`-only return would invite.
#[napi]
pub fn version() -> Result<serde_json::Value> {
    invoke_verb(|ctx, stdout| verbs::version::run(ctx, stdout).map_err(into_napi_io))
}

#[napi(object)]
pub struct NapiExecuteOptions {
    /// Invocation rendered by help, usage, and parser errors.
    pub bin_name: Option<String>,
}

/// Execute any supported package command through the same parser and
/// dispatcher as the standalone `akuapkg` binary.
///
/// Output is intentionally written by the package command itself, preserving
/// its documented JSON/text streams. The numeric result is the package
/// contract's stable exit code, not a Node or shell-process exit status.
#[napi]
pub fn execute(args: Vec<String>, options: Option<NapiExecuteOptions>) -> Result<i32> {
    let bin_name = options
        .and_then(|value| value.bin_name)
        .unwrap_or_else(|| "akuapkg".to_string());
    if bin_name.trim().is_empty() {
        return Err(Error::new(
            Status::InvalidArg,
            "execute binName must not be empty",
        ));
    }
    Ok(akuapkg_cli::entrypoint::run_embedded_from_with_bin_name(
        std::iter::once("embedded".to_string()).chain(args),
        &bin_name,
    )
    .code())
}

/// Point the embedded helm/kustomize engines at a directory holding
/// their `.wasm`/`.cwasm` artifacts. The JS loader calls this with the
/// resolved `@akua-dev/native-engines` directory at module-load time.
///
/// This is the cross-runtime path: setting `AKUA_NATIVE_ENGINES_DIR`
/// via `process.env` reaches `std::env` under Node but not under Bun
/// (Bun doesn't `setenv` on assignment), so the addon takes the dir
/// directly. Must run before the first render — the engine crates
/// cache the resolved bytes on first use.
#[napi]
pub fn set_engines_dir(dir: String) {
    akua_core::set_native_engines_dir(&dir);
}

// ---------------------------------------------------------------------------
// render
// ---------------------------------------------------------------------------

#[napi(object)]
pub struct NapiRenderArgs {
    pub package: String,
    pub inputs: Option<String>,
    pub out: String,
    pub dry_run: Option<bool>,
    pub strict: Option<bool>,
    pub offline: Option<bool>,
    /// Wall-clock cap (Go duration, e.g. `"30s"`, `"5m"`). Maps to
    /// the universal `--timeout` flag; on the SDK side, exposed as
    /// `RenderOptions.timeout`.
    pub timeout: Option<String>,
    /// Hard cap on `pkg.render` composition depth. `BudgetSnapshot`
    /// default (16) when omitted.
    pub max_depth: Option<u32>,
}

#[napi]
pub fn render(args: NapiRenderArgs) -> Result<serde_json::Value> {
    let ctx = render_ctx(&args);
    let package_path = Path::new(&args.package);
    let inputs_path = args.inputs.as_deref().map(Path::new);
    let out_dir = Path::new(&args.out);
    let verb_args = verbs::render::RenderArgs {
        package_path,
        inputs_path,
        out_dir,
        dry_run: args.dry_run.unwrap_or(false),
        stdout_mode: false,
        strict: args.strict.unwrap_or(false),
        offline: args.offline.unwrap_or(false),
        debug: false,
        max_depth: args.max_depth.map(|n| n as usize),
    };
    invoke_verb_with(&ctx, |ctx, stdout| {
        verbs::render::run(ctx, &verb_args, stdout)
            .map_err(|e| into_napi(e.to_structured(), e.exit_code()))
    })
}

/// Build the per-render Context. Forwards `timeout` from the
/// JS-side `RenderOptions`; everything else stays at `Context::json`
/// defaults (the SDK always wants structured output).
fn render_ctx(args: &NapiRenderArgs) -> Context {
    Context {
        timeout: args.timeout.clone(),
        ..Context::json()
    }
}

/// Render a Package and return the multi-document YAML directly,
/// bypassing the on-disk write + summary envelope. Mirrors
/// `akua render --stdout`. The SDK uses this for `renderSource()`
/// where the caller wants raw YAML, not a `RenderSummary`.
#[napi]
pub fn render_to_yaml(args: NapiRenderArgs) -> Result<String> {
    let package_path = Path::new(&args.package);
    let inputs_path = args.inputs.as_deref().map(Path::new);
    let out_dir = Path::new(&args.out);
    let verb_args = verbs::render::RenderArgs {
        package_path,
        inputs_path,
        out_dir,
        dry_run: args.dry_run.unwrap_or(false),
        // Critical: stdout_mode short-circuits the file-writing path
        // and emits raw multi-doc YAML to stdout. Same path
        // `akua render --stdout` uses.
        stdout_mode: true,
        strict: args.strict.unwrap_or(false),
        offline: args.offline.unwrap_or(false),
        debug: false,
        max_depth: args.max_depth.map(|n| n as usize),
    };
    let ctx = render_ctx(&args);
    let mut out = Cursor::new(Vec::new());
    verbs::render::run(&ctx, &verb_args, &mut out)
        .map_err(|e| into_napi(e.to_structured(), e.exit_code()))?;
    let bytes = out.into_inner();
    String::from_utf8(bytes)
        .map_err(|e| Error::from_reason(format!("render output not utf-8: {e}")))
}

// ---------------------------------------------------------------------------
// lint / fmt — single-file pure-compute verbs
// ---------------------------------------------------------------------------

#[napi(object)]
pub struct NapiPackageArgs {
    pub package: String,
}

#[napi]
pub fn lint(args: NapiPackageArgs) -> Result<serde_json::Value> {
    let path = Path::new(&args.package);
    let verb_args = verbs::lint::LintArgs { package_path: path };
    invoke_verb(|ctx, stdout| {
        verbs::lint::run(ctx, &verb_args, stdout)
            .map_err(|e| into_napi(e.to_structured(), e.exit_code()))
    })
}

#[napi(object)]
pub struct NapiFmtArgs {
    pub package: String,
    /// `--check`: exit 1 if the file would change; do not write.
    pub check: Option<bool>,
    /// `--stdout`: print the formatted source instead of writing it.
    pub stdout: Option<bool>,
}

#[napi]
pub fn fmt(args: NapiFmtArgs) -> Result<serde_json::Value> {
    let path = Path::new(&args.package);
    let verb_args = verbs::fmt::FmtArgs {
        package_path: path,
        check: args.check.unwrap_or(false),
        stdout_mode: args.stdout.unwrap_or(false),
    };
    invoke_verb(|ctx, stdout| {
        verbs::fmt::run(ctx, &verb_args, stdout)
            .map_err(|e| into_napi(e.to_structured(), e.exit_code()))
    })
}

// ---------------------------------------------------------------------------
// check — workspace + package together
// ---------------------------------------------------------------------------

#[napi(object)]
pub struct NapiCheckArgs {
    pub workspace: String,
    pub package: Option<String>,
}

#[napi]
pub fn check(args: NapiCheckArgs) -> Result<serde_json::Value> {
    let workspace = Path::new(&args.workspace);
    let pkg_buf;
    let package_path = match &args.package {
        Some(p) => {
            pkg_buf = std::path::PathBuf::from(p);
            pkg_buf.as_path()
        }
        None => {
            pkg_buf = workspace.join("package.k");
            pkg_buf.as_path()
        }
    };
    let verb_args = verbs::check::CheckArgs {
        workspace,
        package_path,
    };
    invoke_verb(|ctx, stdout| {
        verbs::check::run(ctx, &verb_args, stdout)
            .map_err(|e| into_napi(e.to_structured(), e.exit_code()))
    })
}

// ---------------------------------------------------------------------------
// add — insert a dependency into akua.toml
// ---------------------------------------------------------------------------

#[napi(object)]
pub struct NapiAddArgs {
    /// Workspace root containing `akua.toml`. Defaults to `.`.
    pub workspace: Option<String>,

    /// Local alias the dep is keyed under in `[dependencies]`.
    pub name: String,

    /// OCI source URL (e.g. `oci://ghcr.io/foo/charts/bar`).
    pub oci: Option<String>,

    /// Git source URL.
    pub git: Option<String>,

    /// Local filesystem path.
    pub path: Option<String>,

    /// HTTPS Helm-repo URL (pairs with `chart`).
    pub repo: Option<String>,

    /// Chart name within the Helm repo (required with `repo`).
    pub chart: Option<String>,

    /// Version constraint. Required for OCI and Helm-repo deps.
    pub version: Option<String>,

    /// Git tag (alternative to `rev`).
    pub tag: Option<String>,

    /// Git commit SHA (alternative to `tag`).
    pub rev: Option<String>,

    /// Overwrite an existing entry under `name` instead of erroring.
    pub force: Option<bool>,
}

#[napi]
pub fn add(args: NapiAddArgs) -> Result<serde_json::Value> {
    let workspace_str = args.workspace.unwrap_or_else(|| ".".to_string());
    let workspace = Path::new(&workspace_str);
    let name = args.name;
    let source = match (
        args.oci.as_deref(),
        args.git.as_deref(),
        args.path.as_deref(),
        args.repo.as_deref(),
    ) {
        (Some(s), None, None, None) => verbs::add::AddSource::Oci(s),
        (None, Some(s), None, None) => verbs::add::AddSource::Git(s),
        (None, None, Some(s), None) => verbs::add::AddSource::Path(s),
        (None, None, None, Some(r)) => {
            let c = args.chart.as_deref().ok_or_else(|| {
                Error::from_reason("`repo` requires `chart` — pass chart: \"<chart-name>\"")
            })?;
            verbs::add::AddSource::Repo { repo: r, chart: c }
        }
        _ => {
            return Err(Error::from_reason(
                "add: exactly one of `oci`, `git`, `path`, or `repo` must be provided",
            ))
        }
    };
    let verb_args = verbs::add::AddArgs {
        workspace,
        name: &name,
        source,
        version: args.version.as_deref(),
        tag: args.tag.as_deref(),
        rev: args.rev.as_deref(),
        force: args.force.unwrap_or(false),
    };
    invoke_verb(|ctx, stdout| {
        verbs::add::run(ctx, &verb_args, stdout)
            .map_err(|e| into_napi(e.to_structured(), e.exit_code()))
    })
}

// ---------------------------------------------------------------------------
// tree / diff — workspace + chart-comparison verbs
// ---------------------------------------------------------------------------

#[napi(object)]
pub struct NapiWorkspaceArgs {
    pub workspace: String,
}

#[napi]
pub fn tree(args: NapiWorkspaceArgs) -> Result<serde_json::Value> {
    let workspace = Path::new(&args.workspace);
    let verb_args = verbs::tree::TreeArgs { workspace };
    invoke_verb(|ctx, stdout| {
        verbs::tree::run(ctx, &verb_args, stdout)
            .map_err(|e| into_napi(e.to_structured(), e.exit_code()))
    })
}

#[napi(object)]
pub struct NapiDiffArgs {
    pub before: String,
    pub after: String,
}

#[napi]
pub fn diff(args: NapiDiffArgs) -> Result<serde_json::Value> {
    let before = Path::new(&args.before);
    let after = Path::new(&args.after);
    let verb_args = verbs::diff::DiffArgs { before, after };
    invoke_verb(|ctx, stdout| {
        verbs::diff::run(ctx, &verb_args, stdout)
            .map_err(|e| into_napi(e.to_structured(), e.exit_code()))
    })
}

// ---------------------------------------------------------------------------
// export — JSON Schema / OpenAPI emit
// ---------------------------------------------------------------------------

#[napi(object)]
pub struct NapiExportArgs {
    pub package: String,
    /// `"json-schema"` (default) or `"openapi"`.
    pub format: Option<String>,
    /// When set, write the schema to this file instead of stdout. The
    /// CLI verb writes the file AND prints a confirmation; we capture
    /// only the JSON envelope.
    pub out: Option<String>,
}

#[napi]
pub fn export(args: NapiExportArgs) -> Result<serde_json::Value> {
    let package_path = Path::new(&args.package);
    let format = match args.format.as_deref() {
        None | Some("json-schema") => verbs::export::ExportFormat::JsonSchema,
        Some("openapi") => verbs::export::ExportFormat::Openapi,
        Some(other) => {
            return Err(Error::from_reason(format!(
                "unknown format `{other}` (expected `json-schema` or `openapi`)"
            )))
        }
    };
    let out_path;
    let out: Option<&Path> = match &args.out {
        Some(p) => {
            out_path = std::path::PathBuf::from(p);
            Some(out_path.as_path())
        }
        None => None,
    };
    let verb_args = verbs::export::ExportArgs {
        package_path,
        format,
        out,
    };
    invoke_verb(|ctx, stdout| {
        verbs::export::run(ctx, &verb_args, stdout)
            .map_err(|e| into_napi(e.to_structured(), e.exit_code()))
    })
}

// ---------------------------------------------------------------------------
// inspect — Package or tarball introspection
// ---------------------------------------------------------------------------

#[napi(object)]
pub struct NapiInspectArgs {
    /// On-disk Package directory or `package.k`. Mutually exclusive
    /// with `tarball`.
    pub package: Option<String>,
    /// `.tar.gz` Package artifact (e.g. from `akua pack`). Mutually
    /// exclusive with `package`.
    pub tarball: Option<String>,
}

#[napi(object)]
pub struct NapiInspectOciPackageArgs {
    pub oci_ref: String,
    pub tag: String,
    pub auth: Option<HashMap<String, NapiOciRegistryAuth>>,
}

#[napi(object)]
pub struct NapiOciRegistryAuth {
    pub username: Option<String>,
    pub password: Option<String>,
    pub token: Option<String>,
}

#[napi]
pub fn inspect(args: NapiInspectArgs) -> Result<serde_json::Value> {
    let target = match (args.package.as_deref(), args.tarball.as_deref()) {
        (Some(_), Some(_)) => {
            return Err(Error::from_reason(
                "inspect: pass either `package` or `tarball`, not both",
            ));
        }
        (None, None) => {
            return Err(Error::from_reason(
                "inspect: pass either `package` or `tarball`",
            ));
        }
        (Some(p), None) => verbs::inspect::InspectTarget::Package(Path::new(p)),
        (None, Some(t)) => verbs::inspect::InspectTarget::Tarball(Path::new(t)),
    };
    let verb_args = verbs::inspect::InspectArgs { target };
    invoke_verb(|ctx, stdout| {
        verbs::inspect::run(ctx, &verb_args, stdout)
            .map_err(|e| into_napi(e.to_structured(), e.exit_code()))
    })
}

#[napi]
pub fn inspect_oci_package(args: NapiInspectOciPackageArgs) -> Result<serde_json::Value> {
    let creds = oci_registry_auth_store(args.auth)?;
    let output = verbs::inspect::inspect_oci_package_with_creds(&args.oci_ref, &args.tag, &creds)
        .map_err(inspect_oci_package_into_napi)?;
    let ctx = Context::json();
    invoke_verb_with(&ctx, move |ctx, stdout| {
        emit_output(stdout, ctx, &output, |_| Ok(())).map_err(into_napi_io)?;
        Ok(ExitCode::Success)
    })
}

fn inspect_oci_package_into_napi(err: verbs::inspect::InspectError) -> Error {
    if let verbs::inspect::InspectError::OciPull(OciPullError::Transport(
        TransportError::AuthRequired { registry },
    )) = &err
    {
        let mut structured = err.to_structured();
        structured.message = format!(
            "registry `{registry}` rejected auth. Pass explicit credentials in inspectOciPackage({{ auth }}) for this registry."
        );
        return into_napi(structured, err.exit_code());
    }

    into_napi(err.to_structured(), err.exit_code())
}

fn oci_registry_auth_store(
    auth: Option<HashMap<String, NapiOciRegistryAuth>>,
) -> Result<akua_core::oci_auth::CredsStore> {
    let mut creds = akua_core::oci_auth::CredsStore::empty();
    for (registry, entry) in auth.unwrap_or_default() {
        if let Some(token) = entry.token {
            creds
                .entries
                .insert(registry, akua_core::oci_auth::Credentials::Bearer { token });
            continue;
        }
        match (entry.username, entry.password) {
            (Some(username), Some(password)) => {
                creds.entries.insert(
                    registry,
                    akua_core::oci_auth::Credentials::Basic { username, password },
                );
            }
            _ => {
                return Err(Error::new(
                    Status::InvalidArg,
                    format!(
                        "inspectOciPackage auth for `{registry}` must include token or username/password"
                    ),
                ));
            }
        }
    }
    Ok(creds)
}

// ---------------------------------------------------------------------------
// verify — workspace lockfile ↔ manifest integrity
// ---------------------------------------------------------------------------

#[napi]
pub fn verify(args: NapiWorkspaceArgs) -> Result<serde_json::Value> {
    let workspace = Path::new(&args.workspace);
    invoke_verb(|ctx, stdout| {
        verbs::verify::run(ctx, workspace, stdout)
            .map_err(|e| into_napi(e.to_structured(), e.exit_code()))
    })
}

// ---------------------------------------------------------------------------
// vendor — workspace vendor-tree sync / inspection
// ---------------------------------------------------------------------------

/// HTTP basic-auth credential (mirrors
/// [`akua_core::host_auth::BasicAuth`]).
#[napi(object)]
pub struct NapiBasicAuth {
    pub username: String,
    pub password: String,
}

#[napi(object)]
pub struct NapiVendorAddArgs {
    pub workspace: String,
    pub name: String,
    pub plan: Option<bool>,
    /// Credentials for private git remotes, keyed by URL prefix
    /// (longest match wins). Akua never reads ambient credential
    /// files; this is the only way to authenticate from the SDK.
    pub auth: Option<HashMap<String, NapiBasicAuth>>,
}

#[napi]
pub fn vendor_add(args: NapiVendorAddArgs) -> Result<serde_json::Value> {
    let workspace = Path::new(&args.workspace);
    let name = args.name;
    let auth: Option<akua_core::host_auth::HostAuthMap> = args.auth.map(|m| {
        m.into_iter()
            .map(|(k, v)| {
                (
                    k,
                    akua_core::host_auth::BasicAuth {
                        username: v.username,
                        password: v.password,
                    },
                )
            })
            .collect()
    });
    let output = if args.plan.unwrap_or(false) {
        core_vendor::plan_add(workspace, &name, auth.as_ref())
            .map_err(|e| into_napi(e.to_structured(), e.exit_code()))?
    } else {
        core_vendor::add(workspace, &name, auth.as_ref())
            .map_err(|e| into_napi(e.to_structured(), e.exit_code()))?
    };
    let ctx = Context::json();
    invoke_verb_with(&ctx, move |ctx, stdout| {
        emit_output(stdout, ctx, &output, |_| Ok(())).map_err(into_napi_io)?;
        Ok(ExitCode::Success)
    })
}

#[napi]
pub fn vendor_check(args: NapiWorkspaceArgs) -> Result<serde_json::Value> {
    let workspace = Path::new(&args.workspace);
    let output = match core_vendor::check(workspace) {
        Ok(output) => output,
        Err(core_vendor::VendorError::Drift { output }) => output,
        Err(err) => return Err(into_napi(err.to_structured(), err.exit_code())),
    };
    let drift = output.drift;
    let ctx = Context::json();
    invoke_verb_with(&ctx, move |ctx, stdout| {
        emit_output(stdout, ctx, &output, |_| Ok(())).map_err(into_napi_io)?;
        Ok(if drift {
            ExitCode::UserError
        } else {
            ExitCode::Success
        })
    })
}

#[napi]
pub fn vendor_list(args: NapiWorkspaceArgs) -> Result<serde_json::Value> {
    let workspace = Path::new(&args.workspace);
    let output =
        core_vendor::list(workspace).map_err(|e| into_napi(e.to_structured(), e.exit_code()))?;
    let ctx = Context::json();
    invoke_verb_with(&ctx, move |ctx, stdout| {
        emit_output(stdout, ctx, &output, |_| Ok(())).map_err(into_napi_io)?;
        Ok(ExitCode::Success)
    })
}

// ---------------------------------------------------------------------------
// whoami — agent-context introspection
// ---------------------------------------------------------------------------

#[napi]
pub fn whoami() -> Result<serde_json::Value> {
    invoke_verb(|ctx, stdout| verbs::whoami::run(ctx, stdout).map_err(into_napi_io))
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn invoke_verb<F>(run: F) -> Result<serde_json::Value>
where
    F: FnOnce(&Context, &mut Cursor<Vec<u8>>) -> Result<ExitCode>,
{
    invoke_verb_with(&Context::json(), run)
}

fn invoke_verb_with<F>(ctx: &Context, run: F) -> Result<serde_json::Value>
where
    F: FnOnce(&Context, &mut Cursor<Vec<u8>>) -> Result<ExitCode>,
{
    let mut stdout = Cursor::new(Vec::new());
    let exit = run(ctx, &mut stdout)?;
    let bytes = stdout.into_inner();
    if bytes.is_empty() {
        // Every shipping verb writes a JSON envelope under
        // Context::json(). Empty stdout means a verb diverged from
        // that contract — fail loudly so the gap is visible to
        // tests and to JS consumers, not silently masked by a
        // synthetic envelope they couldn't parse anyway.
        return Err(Error::from_reason(format!(
            "akua verb returned exit={exit:?} with empty stdout — every json-mode verb must write an envelope"
        )));
    }
    serde_json::from_slice(&bytes).map_err(|e| {
        Error::from_reason(format!(
            "verb produced non-JSON output (exit={exit:?}): {e}\n\nbytes: {}",
            String::from_utf8_lossy(&bytes)
        ))
    })
}

/// Convert a verb's [`StructuredError`] + [`ExitCode`] into a napi
/// `Error` that preserves both the structured `code` (for fine-grain
/// matching) and the numeric exit code (for SDK error-class routing
/// — `AkuaUserError` / `AkuaSystemError` / etc.). Same envelope the
/// CLI emits to stderr, plus the `exit_code` numeric from the verb.
/// Without this, every JS-side error would collapse to the generic
/// `AkuaError` and lose typed routing.
fn into_napi(structured: StructuredError, exit_code: ExitCode) -> Error {
    let mut body = match serde_json::to_value(&structured) {
        Ok(v) => v,
        Err(_) => return Error::from_reason(structured.message),
    };
    if let Some(obj) = body.as_object_mut() {
        obj.insert("exit_code".to_string(), serde_json::json!(exit_code as i32));
    }
    Error::from_reason(body.to_string())
}

/// Fallback for verbs whose `run()` returns a non-structured error
/// (e.g. `whoami` and `version` return `std::io::Result<ExitCode>`).
/// No structured code to preserve; the message reaches JS as the
/// generic `Error.message`.
fn into_napi_io<E: std::fmt::Display>(err: E) -> Error {
    Error::from_reason(err.to_string())
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------
//
// Bindings are thin pass-throughs to `verbs::*::run`. Tests here focus
// on what THIS layer does that the verb tests don't cover:
//   - Napi*Args → verb args translation
//   - JSON envelope produced by `invoke_verb` (non-empty, parseable)
//   - structured-error envelope (`into_napi`) augmented with `exit_code`
//
// Run with `cargo test -p akua-napi`. Fixtures use a tempdir + the
// minimal-workspace shape (akua.toml + package.k) the SDK tests use.

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    const MINIMAL_PACKAGE_K: &str = r#"
schema Input:
    replicas: int = 2

input: Input = option("input") or Input {}

resources = [{
    apiVersion: "v1"
    kind: "ConfigMap"
    metadata.name: "smoke"
    data.count: str(input.replicas)
}]
"#;

    const MINIMAL_AKUA_TOML: &str = r#"[package]
name = "napi-test"
version = "0.0.1"
edition = "akua.dev/v1alpha1"
"#;

    fn scratch_workspace() -> PathBuf {
        let dir = tempfile::tempdir().unwrap().keep();
        fs::write(dir.join("akua.toml"), MINIMAL_AKUA_TOML).unwrap();
        fs::write(dir.join("package.k"), MINIMAL_PACKAGE_K).unwrap();
        dir
    }

    #[test]
    fn execute_returns_the_package_contract_exit_code() {
        assert_eq!(
            execute(vec!["not-a-command".to_string()], None).unwrap(),
            ExitCode::UserError.code()
        );
    }

    #[test]
    fn version_returns_object_with_version_field() {
        let v = version().unwrap();
        assert!(v.is_object(), "version must return an object envelope");
        assert!(v.get("version").is_some(), "envelope missing `version`");
    }

    #[test]
    fn whoami_returns_agent_context_envelope() {
        let v = whoami().unwrap();
        assert!(v.is_object());
        assert!(v.get("agent_context").is_some(), "missing agent_context");
        assert!(v.get("version").is_some(), "missing version");
    }

    #[test]
    fn lint_returns_issues_array() {
        let ws = scratch_workspace();
        let v = lint(NapiPackageArgs {
            package: ws.join("package.k").to_string_lossy().into_owned(),
        })
        .unwrap();
        assert!(v.is_object());
        assert!(
            v.get("issues").is_some_and(|x| x.is_array()),
            "lint envelope must carry an `issues` array"
        );
    }

    #[test]
    fn fmt_check_mode_does_not_modify_file() {
        let ws = scratch_workspace();
        let pkg = ws.join("package.k");
        let original = fs::read_to_string(&pkg).unwrap();
        let v = fmt(NapiFmtArgs {
            package: pkg.to_string_lossy().into_owned(),
            check: Some(true),
            stdout: Some(false),
        })
        .unwrap();
        assert!(v.is_object());
        assert!(v.get("files").is_some_and(|x| x.is_array()));
        // --check must not write back even if formatted form differs.
        assert_eq!(fs::read_to_string(&pkg).unwrap(), original);
    }

    #[test]
    fn check_returns_status_and_checks_array() {
        let ws = scratch_workspace();
        let v = check(NapiCheckArgs {
            workspace: ws.to_string_lossy().into_owned(),
            package: None,
        })
        .unwrap();
        assert!(v.is_object());
        let status = v.get("status").and_then(|s| s.as_str()).unwrap();
        assert!(matches!(status, "ok" | "fail"));
        assert!(v.get("checks").is_some_and(|x| x.is_array()));
    }

    #[test]
    fn tree_returns_package_and_dependencies_envelope() {
        let ws = scratch_workspace();
        let v = tree(NapiWorkspaceArgs {
            workspace: ws.to_string_lossy().into_owned(),
        })
        .unwrap();
        assert!(v.is_object());
        assert!(v.get("package").is_some());
        let deps = v.get("dependencies").unwrap();
        assert!(deps.is_array());
        // Minimal workspace declares no deps.
        assert_eq!(deps.as_array().unwrap().len(), 0);
    }

    #[test]
    fn diff_two_empty_dirs_reports_no_changes() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let v = diff(NapiDiffArgs {
            before: a.path().to_string_lossy().into_owned(),
            after: b.path().to_string_lossy().into_owned(),
        })
        .unwrap();
        assert!(v.is_object());
        assert_eq!(v["added"].as_array().unwrap().len(), 0);
        assert_eq!(v["removed"].as_array().unwrap().len(), 0);
        assert_eq!(v["changed"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn diff_added_file_surfaces_in_added_bucket() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        fs::write(b.path().join("new.yaml"), "hi\n").unwrap();
        let v = diff(NapiDiffArgs {
            before: a.path().to_string_lossy().into_owned(),
            after: b.path().to_string_lossy().into_owned(),
        })
        .unwrap();
        let added: Vec<&str> = v["added"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|x| x.as_str())
            .collect();
        assert!(
            added.contains(&"new.yaml"),
            "added bucket missing new.yaml: {v}"
        );
    }

    #[test]
    fn export_returns_format_and_schema_envelope() {
        let ws = scratch_workspace();
        let v = export(NapiExportArgs {
            package: ws.join("package.k").to_string_lossy().into_owned(),
            format: None,
            out: None,
        })
        .unwrap();
        assert!(v.is_object());
        assert_eq!(v["format"].as_str().unwrap(), "json-schema");
        let schema = &v["schema"];
        assert!(schema.is_object(), "schema must be an object");
        // The default output is JSON Schema 2020-12.
        assert!(
            schema["$schema"]
                .as_str()
                .map(|s| s.contains("2020-12"))
                .unwrap_or(false),
            "expected JSON Schema 2020-12 dialect: {v}"
        );
    }

    #[test]
    fn export_openapi_format_yields_openapi_3_1() {
        let ws = scratch_workspace();
        let v = export(NapiExportArgs {
            package: ws.join("package.k").to_string_lossy().into_owned(),
            format: Some("openapi".to_string()),
            out: None,
        })
        .unwrap();
        assert_eq!(v["format"].as_str().unwrap(), "openapi");
        assert_eq!(v["schema"]["openapi"].as_str().unwrap(), "3.1.0");
    }

    #[test]
    fn inspect_package_mode_reports_kind_package() {
        let ws = scratch_workspace();
        let v = inspect(NapiInspectArgs {
            package: Some(ws.join("package.k").to_string_lossy().into_owned()),
            tarball: None,
        })
        .unwrap();
        assert_eq!(v["kind"].as_str().unwrap(), "package");
        assert!(v["options"].is_array());
    }

    #[test]
    fn verify_missing_lockfile_returns_structured_error() {
        let ws = scratch_workspace();
        let result = verify(NapiWorkspaceArgs {
            workspace: ws.to_string_lossy().into_owned(),
        });
        // Without a lockfile the verb returns E_LOCK_MISSING. The
        // binding routes this through `into_napi`, which embeds the
        // structured envelope into the napi error message and adds
        // `exit_code`. JS-side `parseNapiError` parses it back.
        let err = result.expect_err("expected E_LOCK_MISSING");
        let msg = err.reason.to_string();
        let envelope: serde_json::Value = serde_json::from_str(&msg)
            .unwrap_or_else(|e| panic!("napi error must be JSON: {e}; raw: {msg}"));
        assert_eq!(envelope["code"].as_str().unwrap(), "E_LOCK_MISSING");
        // The exit_code augmentation is what `into_napi` adds on top
        // of the verb's StructuredError — proves the binding ran.
        assert_eq!(envelope["exit_code"].as_i64().unwrap(), 1);
    }

    #[test]
    fn into_napi_carries_structured_envelope_plus_exit_code() {
        let structured = StructuredError::new("E_TEST", "synthetic");
        let err = into_napi(structured, ExitCode::UserError);
        let body: serde_json::Value = serde_json::from_str(&err.reason).unwrap();
        assert_eq!(body["code"].as_str().unwrap(), "E_TEST");
        assert_eq!(body["message"].as_str().unwrap(), "synthetic");
        assert_eq!(
            body["exit_code"].as_i64().unwrap(),
            ExitCode::UserError as i64
        );
    }
}
