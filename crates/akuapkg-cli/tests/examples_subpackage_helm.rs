//! End-to-end render of `examples/13-subpackage-helm/` — verifies that a
//! sub-package composed via `import pkgs.<alias>` can itself declare and
//! render a local Helm chart (`import charts.nginx`). Regression test for
//! chart context propagating through `pkgs.<alias>` composition.
//!
//! Before the fix in `pkg_render::resolve_child_charts`, a sub-package's
//! `import charts.X` failed with `CannotFindModule` because the child
//! rendered with an empty `ResolvedCharts` regardless of what its own
//! `akua.toml` declared. This test locks down that the full composition
//! chain now works: root → sub-package → Helm chart → single Deployment.
//!
//! Skips cleanly if the render-worker module hasn't been built yet.

#![cfg(all(feature = "cosign-verify", feature = "dev-watch"))]

use std::path::{Path, PathBuf};

use akua_core::{chart_resolver, AkuaManifest, PackageK};
use akuapkg_cli::verbs::render::render_in_worker;

fn example_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("examples/13-subpackage-helm")
}

#[test]
fn renders_subpackage_helm_against_golden() {
    let dir = example_dir();

    // Root manifest declares `webserver` as an Akua-package path dep.
    let manifest = AkuaManifest::load(&dir).expect("load akua.toml");
    let resolved = chart_resolver::resolve(&manifest, &dir).expect("resolve charts");

    // The root has exactly one dep (`webserver`) — an Akua package, not
    // a Helm chart, so the root's `resolved.entries` has one KclModule entry
    // and no HelmChart entries.
    assert_eq!(
        resolved.entries.len(),
        1,
        "root should have exactly one dep (webserver)"
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

    // The webserver sub-package renders one Deployment from the nginx chart.
    assert_eq!(
        rendered.resources.len(),
        1,
        "sub-package + chart should produce one Deployment"
    );

    let deployment = &rendered.resources[0];
    assert_eq!(
        deployment["kind"].as_str(),
        Some("Deployment"),
        "chart emits a Deployment"
    );
    assert_eq!(
        deployment["metadata"]["name"].as_str(),
        Some("web-nginx"),
        "release `web` + chart name `nginx` → `web-nginx`"
    );
    // Namespace from inputs.example.yaml flows through the root Package
    // down into the sub-package and into the Helm release.
    assert_eq!(
        deployment["metadata"]["namespace"].as_str(),
        Some("demo"),
        "namespace from inputs propagated into the Helm release"
    );

    // Byte-equal golden.
    let golden = serde_yaml::from_slice::<serde_yaml::Value>(
        &std::fs::read(dir.join("rendered/000-deployment-web-nginx.yaml")).expect("read golden"),
    )
    .expect("parse golden");
    assert_eq!(
        rendered.resources[0], golden,
        "Deployment drifted from rendered/000-deployment-web-nginx.yaml"
    );
}
