//! Shared multi-document YAML parsing for engine-callable output.
//!
//! Every Kubernetes-shaped rendering engine produces a multi-doc YAML
//! stream — one document per resource, separated by `---`. Parsing it
//! back into typed values is identical across callers (helm today,
//! kustomize next), so the logic lives here.

use serde_json::Value;

/// Parse a multi-document YAML byte slice into one `Value` per doc.
/// Empty separator docs (between resources) are dropped so callers
/// can splat the result directly into `resources`.
///
/// `plugin_name` prefixes error strings so a failure inside
/// `helm.template` looks different from one inside `kustomize.build`
/// when surfaced to a Package author.
pub(crate) fn parse(bytes: &[u8], plugin_name: &str) -> Result<Vec<Value>, String> {
    use serde::de::Deserialize;

    let text =
        std::str::from_utf8(bytes).map_err(|e| format!("{plugin_name}: output not utf-8: {e}"))?;

    let mut out = Vec::new();
    for doc in serde_yaml::Deserializer::from_str(text) {
        let value = Value::deserialize(doc)
            .map_err(|e| format!("{plugin_name}: parsing output as YAML: {e}"))?;
        if is_empty_doc(&value) {
            continue;
        }
        out.push(value);
    }
    Ok(out)
}

fn is_empty_doc(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Object(m) => m.is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multi_doc_into_resource_list() {
        let text = br#"
apiVersion: v1
kind: ConfigMap
metadata:
  name: first
---
apiVersion: v1
kind: Service
metadata:
  name: second
"#;
        let docs = parse(text, "test").expect("parse");
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0]["kind"], "ConfigMap");
        assert_eq!(docs[1]["kind"], "Service");
    }

    #[test]
    fn drops_empty_separator_docs() {
        let text = b"---\napiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: x\n---\n---\n";
        let docs = parse(text, "test").expect("parse");
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0]["metadata"]["name"], "x");
    }

    #[test]
    fn empty_input_produces_empty_list() {
        assert_eq!(parse(b"", "test").unwrap(), Vec::<Value>::new());
        assert_eq!(parse(b"---\n", "test").unwrap(), Vec::<Value>::new());
    }

    #[test]
    fn invalid_utf8_surfaces_prefixed_error() {
        let e = parse(&[0xff, 0xfe, 0xfd], "pluginX").unwrap_err();
        assert!(e.starts_with("pluginX:"), "got: {e}");
        assert!(e.contains("not utf-8"));
    }

    /// A `|-` block scalar whose content contains a paragraph-separator
    /// (empty line) must parse without error and preserve the full value —
    /// the shape of temporal's `server-configmap.yaml`, where
    /// `config_template.yaml: |-` embeds multi-section YAML with bare empty
    /// lines. Locks in that the multi-doc parser handles it.
    ///
    /// NOTE: raw byte string (`br#"..."#`) is required to preserve the indentation
    /// of the block scalar. A non-raw `b"...\n\    persistence:"` would strip
    /// leading whitespace from the continuation line, producing invalid YAML.
    #[test]
    fn block_scalar_with_empty_line_parses_correctly() {
        let text = br#"---
apiVersion: v1
kind: ConfigMap
data:
  config_template.yaml: |-
    log:
      stdout: true
      level: "debug,info"

    persistence:
      defaultStore: default
"#;
        let docs = parse(text, "helm.template").unwrap_or_else(|e| {
            panic!("block scalar with paragraph-separator empty line should parse without error; got: {e}")
        });
        assert_eq!(docs.len(), 1, "expected exactly 1 ConfigMap doc");
        let config_val = docs[0]["data"]["config_template.yaml"]
            .as_str()
            .unwrap_or_else(|| {
                panic!(
                    "config_template.yaml should be a string value; got: {:?}",
                    docs[0]["data"]["config_template.yaml"]
                )
            });
        assert!(
            config_val.contains("persistence"),
            "block scalar content must preserve 'persistence' section after the empty line; \
             got: {config_val:?}"
        );
    }

    /// Regression: real Helm multi-doc output (temporal chart, 59 documents total,
    /// 55 non-empty) must parse without error. The fixture contains block scalars
    /// with embedded shell pipe characters and `sed` patterns.
    #[test]
    fn parses_real_helm_multidoc_output() {
        let fixture = include_bytes!("../tests/fixtures/helm-multidoc.yaml");
        let docs = parse(fixture, "helm.template").unwrap_or_else(|e| {
            panic!("helm.template: failed to parse real helm output: {e}");
        });
        assert!(
            docs.len() >= 50,
            "expected ≥50 non-empty docs from temporal chart, got {}",
            docs.len()
        );
        for (i, doc) in docs.iter().enumerate() {
            assert!(
                doc.is_object(),
                "doc[{i}] is not a YAML mapping (got {doc:?})"
            );
        }
    }
}
