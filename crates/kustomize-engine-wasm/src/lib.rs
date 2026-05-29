//! Embedded kustomize engine.
//!
//! Thin shim over `engine-host-wasm`: holds the kustomize `.cwasm`
//! bytes + the kustomize-specific export-name spec, exposes
//! `render_dir` / `render_tar`. Go source in `../go-src/`; shared
//! sandbox posture + session-reuse rationale in
//! `crates/engine-host-wasm/src/lib.rs`.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use engine_host_wasm::{EngineSpec, SessionSlot};
use serde::{Deserialize, Serialize};

/// Embedded engine bytes — AOT `.cwasm` (default) or source `.wasm`
/// (with `precompile` feature OFF, for `@akua-dev/sdk`'s npm
/// distribution). See helm-engine-wasm for the same pattern.
/// With `embed-engines` OFF, the embedded slot is empty — engines
/// ship via `@akua-dev/native-engines` instead.
#[cfg(all(feature = "precompile", feature = "embed-engines"))]
const KUSTOMIZE_ENGINE_BYTES_EMBEDDED: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/kustomize-engine.cwasm"));
#[cfg(all(not(feature = "precompile"), feature = "embed-engines"))]
const KUSTOMIZE_ENGINE_BYTES_EMBEDDED: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/kustomize-engine.wasm"));
#[cfg(not(feature = "embed-engines"))]
const KUSTOMIZE_ENGINE_BYTES_EMBEDDED: &[u8] = &[];
const IS_PRECOMPILED: bool = cfg!(feature = "precompile");

/// Filename the engine bytes live under when loaded from
/// the `AKUA_NATIVE_ENGINES_DIR` override.
const ENGINE_FILENAME: &str = if cfg!(feature = "precompile") {
    "kustomize-engine.cwasm"
} else {
    "kustomize-engine.wasm"
};

/// Programmatic override for the engines directory, set by the napi
/// loader. Checked before `AKUA_NATIVE_ENGINES_DIR` so it works under
/// runtimes (Bun) that don't propagate `process.env` writes to the OS
/// environment that `std::env` reads.
static ENGINES_DIR_OVERRIDE: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Single env var name across both engine crates — see
/// `helm_engine_wasm::ENV_NATIVE_ENGINES_DIR`. Hardcoded here (not
/// imported) to keep this crate buildable without a direct dep on
/// helm-engine-wasm.
const ENV_NATIVE_ENGINES_DIR: &str = "AKUA_NATIVE_ENGINES_DIR";

/// Set the directory the engine `.wasm`/`.cwasm` is loaded from,
/// overriding `AKUA_NATIVE_ENGINES_DIR`. Must be called before the
/// first engine use: [`engine_bytes`] caches the resolved bytes in a
/// `OnceLock` on first call, so a later `set_engines_dir` won't be
/// observed. The napi loader calls this at module-load time, well
/// before any render.
pub fn set_engines_dir<P: Into<PathBuf>>(dir: P) {
    *ENGINES_DIR_OVERRIDE.write().expect("engines-dir lock") = Some(dir.into());
}

/// Resolve the engines directory: the programmatic override wins over
/// the env var. Returns `None` when neither is set (embedded bytes
/// serve).
fn resolve_engines_dir() -> Option<PathBuf> {
    if let Some(d) = ENGINES_DIR_OVERRIDE
        .read()
        .expect("engines-dir lock")
        .clone()
    {
        return Some(d);
    }
    std::env::var_os(ENV_NATIVE_ENGINES_DIR).map(PathBuf::from)
}

/// Resolve engine bytes once per process. See helm-engine-wasm for
/// the rationale; engines ship via `@akua-dev/native-engines`.
fn engine_bytes() -> &'static [u8] {
    use std::sync::OnceLock;
    static OVERRIDE: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    let slot = OVERRIDE.get_or_init(|| {
        let dir = resolve_engines_dir()?;
        let path = dir.join(ENGINE_FILENAME);
        match std::fs::read(&path) {
            Ok(bytes) if !bytes.is_empty() => Some(bytes),
            _ => None,
        }
    });
    slot.as_deref().unwrap_or(KUSTOMIZE_ENGINE_BYTES_EMBEDDED)
}

const SPEC: EngineSpec = EngineSpec {
    name: "kustomize-engine",
    prefix: "kustomize",
    entry: "kustomize_build",
};

#[derive(Debug, thiserror::Error)]
pub enum KustomizeEngineError {
    #[error(transparent)]
    Host(#[from] engine_host_wasm::EngineHostError),

    #[error("engine: {0}")]
    Engine(String),

    #[error("serializing input: {0}")]
    Serialize(#[from] serde_json::Error),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Serialize)]
struct BuildRequest<'a> {
    overlay_tar_gz_b64: String,
    entrypoint: &'a str,
}

#[derive(Debug, Deserialize)]
struct BuildResponse {
    #[serde(default)]
    yaml: String,
    #[serde(default)]
    error: String,
}

/// Render a kustomize overlay directory. Tars `overlay_dir`'s parent so
/// sibling paths like `../base` resolve correctly, hands the tarball to
/// the WASM engine, returns the rendered multi-doc YAML.
pub fn render_dir(overlay_dir: &Path) -> Result<String, KustomizeEngineError> {
    let entrypoint = overlay_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("overlay")
        .to_string();
    let parent = overlay_dir.parent().ok_or_else(|| {
        KustomizeEngineError::Engine(format!(
            "overlay dir has no parent: {}",
            overlay_dir.display()
        ))
    })?;
    let parent_name = parent
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("pkg")
        .to_string();
    let tar_gz = tar_dir(parent, &parent_name)?;
    let guest_entrypoint = format!("{parent_name}/{entrypoint}");
    render_tar(&tar_gz, &guest_entrypoint)
}

/// Render from an already-tar.gz'd overlay tree. `entrypoint` is the
/// path (inside the tarball) of the directory containing
/// `kustomization.yaml`.
pub fn render_tar(tar_gz: &[u8], entrypoint: &str) -> Result<String, KustomizeEngineError> {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(tar_gz);
    let req = BuildRequest {
        overlay_tar_gz_b64: b64,
        entrypoint,
    };
    let input = serde_json::to_vec(&req)?;
    let output = call_guest(&input)?;
    let resp: BuildResponse = serde_json::from_slice(&output)?;
    if !resp.error.is_empty() {
        return Err(KustomizeEngineError::Engine(resp.error));
    }
    Ok(resp.yaml)
}

fn tar_dir(dir: &Path, name_in_archive: &str) -> Result<Vec<u8>, KustomizeEngineError> {
    use std::io::Write;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    {
        let mut tar = tar::Builder::new(&mut gz);
        tar.follow_symlinks(false);
        tar.append_dir_all(name_in_archive, dir)?;
        tar.finish()?;
    }
    gz.flush()?;
    Ok(gz.finish()?)
}

thread_local! {
    static SESSION: SessionSlot = const { std::cell::RefCell::new(None) };
}

fn call_guest(input: &[u8]) -> Result<Vec<u8>, KustomizeEngineError> {
    SESSION.with(|slot| {
        engine_host_wasm::thread_local_call_with(slot, engine_bytes(), SPEC, input, IS_PRECOMPILED)
            .map_err(KustomizeEngineError::from)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_is_built() -> bool {
        engine_bytes().len() > 1_000_000
    }

    #[test]
    fn set_engines_dir_override_wins_over_env() {
        // Programmatic override must take precedence over (and work
        // without) AKUA_NATIVE_ENGINES_DIR — that's the whole point of
        // the Bun fix, where process.env writes don't reach std::env.
        // We exercise resolve_engines_dir() (pre-cache) rather than
        // engine_bytes() so the OnceLock byte cache can't interfere
        // with sibling tests in the same process.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::env::remove_var(ENV_NATIVE_ENGINES_DIR);
        assert_eq!(
            resolve_engines_dir(),
            None,
            "no override + no env should resolve to None"
        );
        set_engines_dir(&dir);
        assert_eq!(
            resolve_engines_dir().as_deref(),
            Some(dir.as_path()),
            "override must be returned even with the env var unset"
        );
        // Leave the global override clear so other tests (and a real
        // render) aren't pinned at this bogus dir.
        *ENGINES_DIR_OVERRIDE.write().unwrap() = None;
    }

    #[test]
    fn embedded_cwasm_bytes_present_or_placeholder() {
        assert!(
            engine_bytes().is_empty() || engine_bytes().len() > 1_000_000,
            "kustomize-engine.cwasm has suspicious size: {} bytes",
            engine_bytes().len()
        );
    }

    /// With `embed-engines` OFF, the embedded slot must be empty so
    /// the per-platform npm binary doesn't carry the wasm — that's
    /// the load-bearing assumption behind splitting the engines into
    /// `@akua-dev/native-engines`.
    #[test]
    #[cfg(not(feature = "embed-engines"))]
    fn embed_off_means_zero_embedded_bytes() {
        assert!(
            KUSTOMIZE_ENGINE_BYTES_EMBEDDED.is_empty(),
            "embed-engines OFF must produce an empty embed slot, got {} bytes",
            KUSTOMIZE_ENGINE_BYTES_EMBEDDED.len()
        );
    }

    /// With `embed-engines` ON (the default), the embed slot must
    /// be populated unless the build skipped engine compilation
    /// (the `0-byte placeholder` branch in build.rs).
    #[test]
    #[cfg(feature = "embed-engines")]
    fn embed_on_means_nonempty_embedded_bytes() {
        assert!(
            KUSTOMIZE_ENGINE_BYTES_EMBEDDED.is_empty()
                || KUSTOMIZE_ENGINE_BYTES_EMBEDDED.len() > 1_000_000,
            "embed-engines ON should produce empty placeholder OR a real artifact (>1MB), got {} bytes",
            KUSTOMIZE_ENGINE_BYTES_EMBEDDED.len()
        );
    }

    #[test]
    fn renders_minimal_overlay() {
        if !engine_is_built() {
            eprintln!("skipping: kustomize-engine.wasm not built");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let pkg = tmp.path().join("pkg");
        let base = pkg.join("base");
        let overlay = pkg.join("overlay");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&overlay).unwrap();
        std::fs::write(
            base.join("kustomization.yaml"),
            "resources:\n  - configmap.yaml\n",
        )
        .unwrap();
        std::fs::write(
            base.join("configmap.yaml"),
            "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: hello\ndata:\n  greeting: hi\n",
        )
        .unwrap();
        std::fs::write(
            overlay.join("kustomization.yaml"),
            "resources:\n  - ../base\nnamePrefix: prod-\n",
        )
        .unwrap();

        let yaml = render_dir(&overlay).expect("render");
        assert!(yaml.contains("prod-hello"), "rendered: {yaml}");
        assert!(yaml.contains("greeting: hi"), "rendered: {yaml}");
    }
}
