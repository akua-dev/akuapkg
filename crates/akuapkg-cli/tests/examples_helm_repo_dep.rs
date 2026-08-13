//! End-to-end render of `examples/14-helm-repo-dep/` against the live
//! `https://stefanprodan.github.io/podinfo` Helm repository. Proves the
//! classic HTTPS Helm-repo dep source (`repo`/`chart`/`version`) end-to-end:
//! index.yaml fetch, tarball download, sha256 pin, unpack, and render through
//! the wasmtime sandbox.
//!
//! First run fetches the podinfo `.tgz` into `~/.cache/akua/helm/<sha256>/`;
//! subsequent runs (including CI runs that share the cache) are cache hits
//! and skip the network. The committed `akua.lock` digest pins the exact
//! tarball, so a registry-side change fails fast instead of silently
//! picking up new bytes.
//!
//! Skips cleanly if the render-worker module hasn't been built yet.

#![cfg(all(feature = "cosign-verify", feature = "dev-watch"))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use akua_core::chart_resolver::ResolverOptions;
use akua_core::lock_file::AkuaLock;
use akua_core::{chart_resolver, AkuaManifest, PackageK};
use akuapkg_cli::verbs::render::render_in_worker;

fn example_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("examples/14-helm-repo-dep")
}

#[test]
// Live fetch from stefanprodan.github.io/podinfo; cold runs require a
// network round-trip to download index.yaml + podinfo-6.12.0.tgz.
// Gated off by default so flakes don't shadow real test failures; run
// explicitly via `cargo test -- --include-ignored` or `task
// release:validate` (which already passes that flag).
#[ignore = "online: fetches from stefanprodan.github.io/podinfo; sensitive to network availability"]
fn renders_helm_repo_dep_podinfo() {
    let dir = example_dir();

    let manifest = AkuaManifest::load(&dir).expect("load akua.toml");
    assert!(
        manifest.dependencies.contains_key("podinfo"),
        "example 14 must declare the podinfo dep"
    );

    // Mirror the production `akuapkg render` flow: load akua.lock for digest
    // pinning, run the resolver online so first-time fetches populate
    // `~/.cache/akua/helm/<sha256>/podinfo/` from the Helm repository.
    let lock = AkuaLock::load(&dir).expect("load akua.lock");
    let expected_digests: BTreeMap<String, String> = lock
        .packages
        .into_iter()
        .filter(|p| p.source.starts_with("helm+"))
        .map(|p| (p.name, p.digest))
        .collect();
    let opts = ResolverOptions {
        offline: false,
        cache_root: None,
        expected_digests,
        cosign_public_key_pem: None,
        reject_replace: false,
        auth: None,
    };
    let resolved =
        chart_resolver::resolve_with_options(&manifest, &dir, &opts).expect("resolve podinfo");
    let podinfo = resolved.entries.get("podinfo").expect("podinfo entry");
    assert_eq!(
        podinfo.kind,
        chart_resolver::PackageKind::HelmChart,
        "helm-repo dep must be detected as HelmChart"
    );
    assert!(podinfo.abs_path.is_absolute());
    // Marker file confirms the resolver unpacked to the chart root.
    assert!(
        podinfo.abs_path.join("Chart.yaml").is_file(),
        "Chart.yaml missing at resolved path {}",
        podinfo.abs_path.display()
    );

    let package = PackageK::load(&dir.join("package.k")).expect("load package.k");
    // podinfo renders cleanly with empty inputs — no inputs file needed.
    let inputs = serde_yaml::Value::Mapping(Default::default());

    let rendered = match render_in_worker(
        &package,
        &inputs,
        &resolved,
        false,
        akua_core::kcl_plugin::BudgetSnapshot::default(),
        akua_core::kcl_plugin::ResolverContext::default(),
    ) {
        Ok(r) => r,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("worker module wasn't compiled") {
                eprintln!("skipping: {msg}");
                return;
            }
            panic!("render failed: {e}");
        }
    };

    // podinfo 6.12.0 with default values emits: Deployment, Service, plus
    // test pods (names include a random suffix). Assert structurally — no
    // byte-golden, since the test-pod names are not deterministic.
    assert!(
        rendered.resources.len() >= 2,
        "expected at least Deployment + Service, got {} resources",
        rendered.resources.len()
    );

    let by_kind = |kind: &str| {
        rendered
            .resources
            .iter()
            .find(|r| r["kind"].as_str() == Some(kind))
    };

    let deploy = by_kind("Deployment").expect("podinfo must emit a Deployment");
    let deploy_name = deploy["metadata"]["name"].as_str().unwrap_or("");
    assert!(
        deploy_name.contains("podinfo"),
        "Deployment name should contain 'podinfo', got '{deploy_name}'"
    );

    let svc = by_kind("Service").expect("podinfo must emit a Service");
    let svc_name = svc["metadata"]["name"].as_str().unwrap_or("");
    assert!(
        svc_name.contains("podinfo"),
        "Service name should contain 'podinfo', got '{svc_name}'"
    );
}
