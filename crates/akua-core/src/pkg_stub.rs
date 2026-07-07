//! Synthesize import-only stubs for Akua-package deps.
//!
//! `import upstream` against a real Akua package runs upstream's
//! module-level body at import time — `input: Input = ctx.input()`
//! reads the consumer's `option("input")` against upstream's schema
//! and panics inside KCL's `type_pack_and_check` when shapes diverge.
//!
//! Mirroring how `import charts.webapp` reaches a synthesized
//! `Chart`/`Values` + `webapp.template(...)` shape, we emit a stub
//! `<alias>.k` per Akua-package dep containing the upstream's
//! `import` and `schema` declarations + a `render` lambda that
//! dispatches to `kcl_plugin.pkg.render` with the alias hardcoded.
//! Stubs mount at `/akua-pkgs` inside the worker; the consumer writes
//!
//! ```kcl
//! import pkgs.upstream as upstream
//! _up = upstream.render(upstream.Input { ... })
//! ```
//!
//! `pkg.render` itself is unaffected — its handler still loads the
//! real `package.k` from disk and renders it through `PackageK::render`.
//! The stub is for compile-time type reach only.

/// Textually extract schema declarations from a `package.k` source.
///
/// Keeps imports referenced by surviving schema type surfaces and
/// every `schema NAME:` block (body recognised by indentation; the
/// block ends at the next non-blank non-indented non-comment line).
/// Drops top-level assignments and free expressions — those are the
/// bodies that would otherwise execute at import time.
///
/// Best-effort; does not parse KCL. Relies on the indentation
/// convention every Package.k follows. The resulting stub still goes
/// through KCL's parser when the consumer imports it; malformed input
/// surfaces as a normal compile error.
pub fn extract_schemas(source: &str) -> String {
    let source = crate::package_k::strip_akua_decorators(source);
    let mut imports = Vec::new();
    let mut schemas = String::new();
    let mut in_schema = false;

    for line in source.lines() {
        let trimmed_start = line.trim_start();
        let is_blank = trimmed_start.is_empty();
        let is_indented = line.starts_with(' ') || line.starts_with('\t');
        let is_comment = trimmed_start.starts_with('#');

        if in_schema {
            if is_blank || is_indented || is_comment {
                schemas.push_str(line);
                schemas.push('\n');
                continue;
            }
            in_schema = false;
        }

        if is_blank {
            // Compress runs of blank lines into one separator.
            if !schemas.ends_with("\n\n") {
                schemas.push('\n');
            }
            continue;
        }

        if !is_indented {
            if trimmed_start.starts_with("import ") {
                // `charts.*` imports are per-render synthetic modules that
                // only exist in the render context of the package that
                // declares the dep — they are never available in the
                // consumer's stub-compilation context. Drop them: chart
                // imports are only used in body code (`resources = …`),
                // never in schema type definitions, so the stub doesn't
                // need them for type-checking on the consumer side.
                if trimmed_start.starts_with("import charts.") {
                    continue;
                }
                if let Some(alias) = imported_name(trimmed_start) {
                    imports.push((line, alias));
                }
                continue;
            }
            if trimmed_start.starts_with("schema ") || trimmed_start.starts_with("protocol ") {
                in_schema = true;
                schemas.push_str(line);
                schemas.push('\n');
                continue;
            }
        }
        // Top-level assignments, expressions, decorator-only lines:
        // drop. Schemas + imports are the only things that survive.
    }

    let mut out = String::new();
    for (line, alias) in imports {
        if contains_module_reference(&schemas, alias) {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !out.is_empty() && !schemas.trim().is_empty() {
        out.push('\n');
    }
    out.push_str(&schemas);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn imported_name(import_line: &str) -> Option<&str> {
    let without_comment = import_line.split('#').next()?.trim();
    let import_target = without_comment.strip_prefix("import ")?.trim();
    if let Some((_, alias)) = import_target.rsplit_once(" as ") {
        return alias.split_whitespace().next();
    }
    import_target
        .split_whitespace()
        .next()
        .and_then(|module| module.rsplit('.').next())
}

fn contains_module_reference(source: &str, identifier: &str) -> bool {
    let mut index = 0;
    while index < source.len() {
        let rest = &source[index..];
        if rest.starts_with('#') {
            index += rest.find('\n').unwrap_or(rest.len());
            continue;
        }
        if let Some(quote) = rest.chars().next().filter(|ch| *ch == '"' || *ch == '\'') {
            index = skip_string_literal(source, index, quote);
            continue;
        }
        if rest.starts_with(identifier) {
            let before = source[..index].chars().next_back();
            let after = source[index + identifier.len()..].chars().next();
            if !is_identifier_char(before) && after == Some('.') {
                return true;
            }
        }
        index += rest.chars().next().map(char::len_utf8).unwrap_or(1);
    }
    false
}

fn is_identifier_char(ch: Option<char>) -> bool {
    ch.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn skip_string_literal(source: &str, start: usize, quote: char) -> usize {
    let quote_len = quote.len_utf8();
    let quote_text = &source[start..start + quote_len];
    let rest = &source[start..];
    if rest.starts_with(&quote_text.repeat(3)) {
        return rest[3 * quote_len..]
            .find(&quote_text.repeat(3))
            .map(|offset| start + 3 * quote_len + offset + 3 * quote_len)
            .unwrap_or(source.len());
    }

    let mut escaped = false;
    for (offset, ch) in rest[quote_len..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == quote {
            return start + quote_len + offset + quote_len;
        }
    }
    source.len()
}

/// Compose the full stub module body: extracted schemas + a `render`
/// lambda hardcoded with `alias` so callers write
/// `upstream.render(upstream.Input { ... })`. The lambda's `inputs`
/// parameter is typed as `Input`; KCL's schema-to-dict coercion
/// inside the lambda body is fine even though the consumer-side
/// `pkg.Render.inputs: {str:}` rejects bare schema instances at the
/// top-level call site.
///
/// When the upstream package has no `Input` schema (rare — most
/// real Packages declare one), the lambda falls back to a
/// `{str:}` input shape so the stub still compiles.
pub fn build_stub_module(alias: &str, source: &str) -> String {
    let schemas = extract_schemas(source);
    let has_input = source_declares_input_schema(&schemas);
    let mut out = String::new();
    out.push_str("# Akua-package stub. Auto-generated; do not edit.\n");
    out.push_str("import akua.pkg as _pkg\n\n");
    out.push_str(&schemas);
    if !out.ends_with("\n\n") {
        out.push('\n');
    }
    // No default — `Input {}` would fail at lambda-definition time
    // when upstream has required fields. Inside the lambda body, KCL
    // coerces a typed `Input` to the `{str:}` shape that
    // `_pkg.Render.inputs` expects (same coercion helm.template uses
    // for typed `Values`).
    let param_ty = if has_input { "Input" } else { "{str:}" };
    out.push_str(&format!(
        "render = lambda inputs: {param_ty} -> [{{str:}}] {{\n    \
            _flat = {{**inputs}}\n    \
            _pkg.render(_pkg.Render {{\n        \
                package = \"{alias}\"\n        \
                inputs = _flat\n    \
            }})\n\
        }}\n"
    ));
    out
}

fn source_declares_input_schema(stub_source: &str) -> bool {
    stub_source
        .lines()
        .any(|line| line.trim_start().starts_with("schema Input"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_schema_blocks_and_schema_referenced_imports() {
        let src = r#"
import akua.ctx
import types.common as common

schema Input:
    """The thing."""
    name: str
    placement: common.Placement
    replicas: int = 2

input: Input = ctx.input()

resources = [{"foo": "bar"}]
"#;
        let stub = extract_schemas(src);
        assert!(stub.contains("import types.common as common"));
        assert!(!stub.contains("import akua.ctx"));
        assert!(stub.contains("schema Input:"));
        assert!(stub.contains("name: str"));
        assert!(stub.contains("placement: common.Placement"));
        assert!(stub.contains("replicas: int = 2"));
        assert!(!stub.contains("ctx.input"));
        assert!(!stub.contains("resources"));
    }

    #[test]
    fn handles_check_blocks() {
        let src = r#"
schema Input:
    replicas: int = 1

    check:
        replicas >= 1, "at least one"

input: Input = {}
"#;
        let stub = extract_schemas(src);
        assert!(stub.contains("check:"));
        assert!(stub.contains("replicas >= 1"));
        assert!(!stub.contains("input: Input"));
    }

    #[test]
    fn keeps_multiple_schemas() {
        let src = r#"
schema A:
    a: str

schema B:
    b: int

something = A {a = "x"}
"#;
        let stub = extract_schemas(src);
        assert!(stub.contains("schema A:"));
        assert!(stub.contains("schema B:"));
        assert!(!stub.contains("something"));
    }

    #[test]
    fn empty_or_body_only_source_yields_blank_stub() {
        let stub = extract_schemas("resources = []\n");
        assert_eq!(stub.trim(), "");
    }

    #[test]
    fn build_stub_module_emits_render_lambda_typed_to_input() {
        let src = r#"
schema Input:
    name: str

input: Input = ctx.input()
"#;
        let stub = build_stub_module("upstream", src);
        assert!(stub.contains("import akua.pkg as _pkg"));
        assert!(stub.contains("schema Input:"));
        assert!(stub.contains("render = lambda inputs: Input -> [{str:}]"));
        assert!(stub.contains("_pkg.render(_pkg.Render {"));
        assert!(stub.contains("package = \"upstream\""));
        assert!(!stub.contains("ctx.input"));
    }

    #[test]
    fn build_stub_module_falls_back_to_dict_when_no_input_schema() {
        let src = "schema Other:\n    x: int\n";
        let stub = build_stub_module("upstream", src);
        assert!(stub.contains("render = lambda inputs: {str:} -> [{str:}]"));
    }

    /// `import charts.*` lines must be stripped from the stub because the
    /// synthesized `charts` package only exists in the render context of the
    /// package that declares the chart dep — it is never available in the
    /// consumer's stub-compilation context. A sub-package that does
    /// `import charts.nginx` must not carry that import into its stub, or
    /// the consumer's root render will fail with `CannotFindModule charts.nginx`.
    /// Chart imports only appear in body code (`resources = c.template(…)`),
    /// never in schema type definitions, so the stub doesn't lose any
    /// type information by dropping them.
    #[test]
    fn strips_charts_import_from_stub() {
        let src = r#"
import akua.ctx
import charts.nginx as c
import types.common as common

schema Input:
    namespace: str = "demo"
    placement: common.Placement

input: Input = ctx.input()

resources = c.template(c.TemplateOpts { namespace = input.namespace })
"#;
        let stub = extract_schemas(src);
        // The chart import must be dropped — it's a per-render synthetic
        // module not available in the consumer's context.
        assert!(
            !stub.contains("import charts."),
            "charts.* import must not appear in stub: {stub}"
        );
        // Schema-referenced imports and schema blocks survive.
        assert!(
            stub.contains("import types.common as common"),
            "schema import survives"
        );
        assert!(
            !stub.contains("import akua.ctx"),
            "body-only import is dropped"
        );
        assert!(stub.contains("schema Input:"), "Input schema survives");
        assert!(stub.contains("namespace: str"), "schema field survives");
    }

    #[test]
    fn strips_ui_decorators_from_stub_schemas() {
        let src = r#"
import akua.ui as ui

schema Input:
    @ui(order=10, group="Basics")
    name: str

resources = []
"#;
        let stub = extract_schemas(src);
        assert!(stub.contains("schema Input:"));
        assert!(stub.contains("name: str"));
        assert!(
            !stub.contains("@ui("),
            "render stubs must not carry akua UI decorators"
        );
        assert!(
            !stub.contains("import akua.ui"),
            "unused decorator-only imports must not survive in render stubs"
        );
    }

    #[test]
    fn drops_body_only_imports_from_stub_schemas() {
        let src = r#"
import k8s.api.apps.v1 as apps
import k8s.api.core.v1 as corev1

schema Input:
    name: str

deployment = apps.Deployment {}
resources = [deployment]
"#;
        let stub = extract_schemas(src);
        assert!(stub.contains("schema Input:"));
        assert!(stub.contains("name: str"));
        assert!(
            !stub.contains("import k8s.api.apps.v1"),
            "body-only imports must not force consumers to resolve upstream render deps"
        );
        assert!(
            !stub.contains("import k8s.api.core.v1"),
            "unused imports must not force consumers to resolve upstream render deps"
        );
    }

    #[test]
    fn preserves_imports_used_by_stub_schemas() {
        let src = r#"
import types.common as common
import k8s.api.apps.v1 as apps

schema Input:
    placement: common.Placement

resources = [apps.Deployment {}]
"#;
        let stub = extract_schemas(src);
        assert!(stub.contains("import types.common as common"));
        assert!(!stub.contains("import k8s.api.apps.v1"));
        assert!(stub.contains("placement: common.Placement"));
    }

    #[test]
    fn ignores_import_alias_mentions_that_are_not_module_type_references() {
        let src = r#"
import k8s.api.apps.v1 as apps

schema Input:
    """Mention apps.Deployment in docs without needing the import."""
    # apps.StatefulSet is only a comment example.
    apps: str

resources = [apps.Deployment {}]
"#;
        let stub = extract_schemas(src);
        assert!(stub.contains("apps: str"));
        assert!(!stub.contains("import k8s.api.apps.v1"));
    }
}
