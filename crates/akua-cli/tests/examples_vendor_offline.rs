//! End-to-end render of `examples/12-vendor-offline/` — verifies the
//! offline-render contract: with `.akua/vendor/upstream/` committed and
//! the canonical `path` source GC'd, render still produces the expected
//! ConfigMap because the resolver's vendor-first lookup is universal
//! across dep types (path / oci / git).
//!
//! Skips cleanly if `helm-engine.wasm` or the render-worker module
//! haven't been built yet.

#![cfg(all(feature = "cosign-verify", feature = "dev-watch"))]

use std::path::{Path, PathBuf};

use akua_cli::verbs::render::render_in_worker;
use akua_core::{chart_resolver, AkuaManifest, PackageK};

fn example_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("examples/12-vendor-offline")
}

#[test]
fn renders_vendor_offline_against_golden() {
    let dir = example_dir();

    // The example commits `.akua/vendor/upstream/` and `akua.lock` but
    // NOT the canonical `upstream-chart/` source — this is the offline-
    // render guarantee. If the resolver tried to read the canonical
    // source it would fail; success here proves the vendor-first path.
    assert!(
        !dir.join("upstream-chart").exists(),
        "canonical source must be absent for this example to demonstrate offline render"
    );
    assert!(
        dir.join(".akua/vendor/upstream").is_dir(),
        "vendor tree must be committed"
    );

    let manifest = AkuaManifest::load(&dir).expect("load akua.toml");
    let resolved = chart_resolver::resolve(&manifest, &dir).expect("resolve via vendor tree");
    let upstream = resolved.entries.get("upstream").expect("upstream resolved");
    assert!(
        upstream.abs_path.ends_with(".akua/vendor/upstream"),
        "resolver must read from .akua/vendor/upstream, got {}",
        upstream.abs_path.display()
    );

    let package = PackageK::load(&dir.join("package.k")).expect("load package.k");
    let inputs = serde_yaml::from_slice::<serde_yaml::Value>(
        &std::fs::read(dir.join("inputs.example.yaml")).expect("read inputs.example.yaml"),
    )
    .expect("parse inputs");

    let rendered = match render_in_worker(
        &package,
        &inputs,
        &resolved,
        false,
        akua_core::kcl_plugin::BudgetSnapshot::default(),
    ) {
        Ok(r) => r,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("helm-engine.wasm not built")
                || msg.contains("worker module wasn't compiled")
            {
                eprintln!("skipping: {msg}");
                return;
            }
            panic!("render failed: {e}");
        }
    };

    assert_eq!(
        rendered.resources.len(),
        1,
        "vendored chart emits one ConfigMap"
    );

    let golden = serde_yaml::from_slice::<serde_yaml::Value>(
        &std::fs::read(dir.join("rendered/000-configmap-vendored-vendored.yaml"))
            .expect("read golden"),
    )
    .expect("parse golden");
    assert_eq!(
        rendered.resources[0], golden,
        "rendered ConfigMap drifted from rendered/000-configmap-vendored-vendored.yaml"
    );
}
