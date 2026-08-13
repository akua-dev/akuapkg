//! Regression test for union-typed `values.schema.json` fields.
//!
//! A Helm chart whose `values.schema.json` declares a field as a
//! multi-type union with a default whose JSON type matches only one
//! member (`{"type":["string","integer","null"],"default":8080}`) used
//! to generate KCL `port: str = 8080` — a contradictory annotation
//! that aborted the wasm evaluator at schema instantiation, even when
//! the Package passed `values = {}`. The fix emits a real union
//! (`port?: int | str = 8080`).
//!
//! This renders the fixture end-to-end through the worker (the only
//! layer that surfaces the abort; parse-only tests can't) and asserts
//! a manifest comes back. Skips cleanly if the engine/worker modules
//! haven't been built.

#![cfg(all(feature = "cosign-verify", feature = "dev-watch"))]

use std::path::{Path, PathBuf};

use akua_core::{chart_resolver, AkuaManifest, PackageK, RenderedPackage};
use akuapkg_cli::verbs::render::render_in_worker;

fn fixture_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{name}"))
}

fn render_fixture(name: &str, regression_hint: &str) -> Option<RenderedPackage> {
    let dir = fixture_dir(name);

    let manifest = AkuaManifest::load(&dir).expect("load akua.toml");
    let resolved = chart_resolver::resolve(&manifest, &dir).expect("resolve charts");

    let package = PackageK::load(&dir.join("package.k")).expect("load package.k");
    let inputs = serde_yaml::Value::Null;

    match render_in_worker(
        &package,
        &inputs,
        &resolved,
        false,
        akua_core::kcl_plugin::BudgetSnapshot::default(),
        akua_core::kcl_plugin::ResolverContext::default(),
    ) {
        Ok(r) => Some(r),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("helm-engine.wasm not built")
                || msg.contains("worker module wasn't compiled")
            {
                eprintln!("skipping: {msg}");
                return None;
            }
            panic!("render failed (regression — {regression_hint}): {e}");
        }
    }
}

#[test]
fn renders_chart_with_union_typed_schema_default() {
    // No public inputs; an empty inputs doc exercises the schema's own
    // default — the path that used to abort.
    let Some(rendered) = render_fixture("helm-union-schema", "union schema aborted evaluator")
    else {
        return;
    };

    assert_eq!(rendered.resources.len(), 1, "chart emits one ConfigMap");
    assert_eq!(
        rendered.resources[0]["kind"].as_str(),
        Some("ConfigMap"),
        "expected a ConfigMap, got {:?}",
        rendered.resources[0]
    );
    // The default (8080) flows through; helm `| quote` stringifies it.
    assert_eq!(
        rendered.resources[0]["data"]["port"].as_str(),
        Some("8080"),
        "schema default did not flow through: {:?}",
        rendered.resources[0]
    );
}

#[test]
fn renders_chart_with_contradictory_schema_defaults_omitted() {
    let Some(rendered) = render_fixture(
        "helm-contradictory-schema-defaults",
        "contradictory values.schema.json defaults aborted evaluator",
    ) else {
        return;
    };

    assert_eq!(rendered.resources.len(), 1, "chart emits one ConfigMap");
    assert_eq!(
        rendered.resources[0]["kind"].as_str(),
        Some("ConfigMap"),
        "expected a ConfigMap, got {:?}",
        rendered.resources[0]
    );
    assert_eq!(
        rendered.resources[0]["data"]["safe"].as_str(),
        Some("rendered"),
        "matching schema default did not flow through: {:?}",
        rendered.resources[0]
    );
}
