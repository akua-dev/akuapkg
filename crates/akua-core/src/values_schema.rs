//! Convert a chart's `values.schema.json` (JSON Schema) into a
//! typed KCL `schema Values` declaration.
//!
//! Helm charts ship with an optional `values.schema.json` at the
//! chart root, used by `helm install --validate` to reject bad
//! `values.yaml` inputs. When present, we generate a KCL mirror so
//! Package authors get the same shape under their IDE / LSP:
//!
//! ```kcl
//! import charts.nginx as nginx
//!
//! _values = nginx.Values {
//!     replicaCount = 3
//!     image = nginx.ValuesImage { tag = "1.27" }
//! }
//! helm.template(helm.Template { chart = nginx.path, values = _values })
//! ```
//!
//! ## Scope
//!
//! gets the common shapes: objects, primitives,
//! arrays, enums. Deferred:
//!
//! - `$ref` (needs a two-pass resolver)
//! - `allOf` / `oneOf` / `anyOf` (no clean KCL mapping short of
//!   generated union types)
//! - `pattern` / `format` validation (would need KCL `check:` blocks)
//! - `additionalProperties: false` (KCL is strict-by-default anyway)
//!
//! Unknown shapes collapse to `any` — the author can override field
//! by field if the generated schema isn't tight enough.

use serde::Deserialize;

/// Input JSON Schema — we model only the subset we handle. Other
/// keywords (`pattern`, `format`, `allOf`, …) are silently ignored
/// on a best-effort basis; stricter validation lives upstream in
/// helm's own `--validate`.
#[derive(Debug, Deserialize)]
struct JsonSchema {
    #[serde(default, rename = "type")]
    ty: Option<TypeSpec>,

    #[serde(default)]
    properties: std::collections::BTreeMap<String, JsonSchema>,

    #[serde(default)]
    required: Vec<String>,

    #[serde(default)]
    items: Option<Box<JsonSchema>>,

    #[serde(default)]
    default: Option<serde_json::Value>,

    #[serde(default, rename = "enum")]
    enum_values: Option<Vec<serde_json::Value>>,

    /// Trailing docstring; surfaced as KCL field doc.
    #[serde(default)]
    description: Option<String>,
}

/// `type:` in JSON Schema can be a string or an array (union).
/// The string form maps to one KCL built-in. The array form: a
/// `"null"` member makes the field optional (the `["string","null"]`
/// optionality idiom); the remaining non-null members map to a KCL
/// union annotation (`int | str`) when there's more than one, or a
/// single built-in when there's one.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TypeSpec {
    Single(String),
    Union(Vec<String>),
}

impl TypeSpec {
    fn primary(&self) -> Option<&str> {
        match self {
            TypeSpec::Single(s) => Some(s.as_str()),
            TypeSpec::Union(v) => v.iter().map(String::as_str).find(|t| *t != "null"),
        }
    }

    /// Maps declared type members in source order (including `"null"`).
    fn find_map_member<T>(&self, mut f: impl FnMut(&str) -> Option<T>) -> Option<T> {
        match self {
            TypeSpec::Single(s) => f(s),
            TypeSpec::Union(v) => v.iter().find_map(|member| f(member)),
        }
    }

    /// True when the union admits `"null"` — JSON Schema's way of
    /// expressing optionality, which KCL models as a non-required field.
    fn is_nullable(&self) -> bool {
        matches!(self, TypeSpec::Union(v) if v.iter().any(|t| t == "null"))
    }
}

/// Generated KCL source. The caller writes it to the chart's
/// per-render module next to `path` / `sha256`.
#[derive(Debug, Clone, Default)]
pub struct GeneratedKcl {
    /// Top-level schema declarations, in dependency order. The root
    /// schema is always named `Values`; nested object schemas are
    /// named `Values<Path>` (e.g. `ValuesImage`, `ValuesImageTag`).
    pub source: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ValuesSchemaError {
    #[error("values.schema.json not valid JSON: {0}")]
    Parse(#[from] serde_json::Error),

    /// A `properties` key is not a legal KCL identifier. Property names
    /// are emitted verbatim into the generated `schema` body, so a key
    /// that isn't `[A-Za-z_][A-Za-z0-9_]*` is rejected rather than
    /// emitted — a crafted key (e.g. containing a newline + statement)
    /// could otherwise inject KCL into the consumer's signed output.
    #[error("values.schema.json property name is not a valid KCL identifier: {name:?}")]
    InvalidPropertyName { name: String },
}

/// Convert `values.schema.json` bytes to a KCL `schema Values` block
/// plus any nested supporting schemas. Root schema is always named
/// `Values`; callers prefix with `<ChartName>Values` themselves if
/// they want a namespaced shape.
///
/// Returns an empty [`GeneratedKcl`] when the input schema is not
/// an object — helm's `values.yaml` is always a dict, so a non-
/// object root schema is a defect in the chart; we surface it as
/// "no typed schema generated" rather than erroring.
pub fn generate_from_bytes(bytes: &[u8]) -> Result<GeneratedKcl, ValuesSchemaError> {
    let schema: JsonSchema = serde_json::from_slice(bytes)?;
    generate(&schema)
}

fn generate(root: &JsonSchema) -> Result<GeneratedKcl, ValuesSchemaError> {
    let primary = root.ty.as_ref().and_then(TypeSpec::primary).unwrap_or("");
    if primary != "object" {
        return Ok(GeneratedKcl::default());
    }
    let mut gen = SchemaGen::default();
    gen.emit_object(root, "Values")?;
    Ok(GeneratedKcl {
        source: gen.finish(),
    })
}

#[derive(Default)]
struct SchemaGen {
    /// Schemas produced so far, in emit order. Nested objects append
    /// after the parent so a forward-referencing root schema is OK
    /// as long as KCL's parser does full-module resolution (it does).
    out: Vec<String>,
}

impl SchemaGen {
    fn emit_object(&mut self, schema: &JsonSchema, name: &str) -> Result<(), ValuesSchemaError> {
        let required: std::collections::HashSet<&str> =
            schema.required.iter().map(String::as_str).collect();

        let mut body = String::new();
        body.push_str(&format!("schema {name}:\n"));
        if let Some(doc) = schema.description.as_deref() {
            body.push_str(&format_docstring(doc, 4));
            body.push('\n');
        }

        if schema.properties.is_empty() {
            // No declared fields — KCL requires at least one statement
            // in a schema body. Emit a passthrough wildcard dict
            // field; callers can still construct an empty `{}`.
            body.push_str("    _: any = None\n\n");
            self.out.push(body);
            return Ok(());
        }

        for (prop_name, prop_schema) in &schema.properties {
            self.emit_field(&mut body, name, prop_name, prop_schema, &required)?;
        }
        body.push('\n');
        self.out.push(body);
        Ok(())
    }

    fn emit_field(
        &mut self,
        out: &mut String,
        parent_name: &str,
        prop_name: &str,
        prop_schema: &JsonSchema,
        required: &std::collections::HashSet<&str>,
    ) -> Result<(), ValuesSchemaError> {
        // Property names are emitted verbatim into the schema body. A
        // key that isn't a legal KCL identifier (one with a newline,
        // `:`, etc.) could inject statements into the consumer's signed
        // output, so reject it rather than emit it raw. The downstream
        // KCL parser is not a defense here — valid-but-injected KCL
        // parses fine.
        if !is_kcl_identifier(prop_name) {
            return Err(ValuesSchemaError::InvalidPropertyName {
                name: prop_name.to_string(),
            });
        }

        let is_required = required.contains(prop_name);
        let nested_name = format!("{parent_name}{}", pascal_case(prop_name));
        let ty = self.render_type(&nested_name, prop_schema)?;

        // A nullable type (`["...","null"]`) means the field may be
        // absent; KCL models that as an optional field regardless of
        // whether JSON Schema marked it required or gave it a default.
        let nullable = prop_schema
            .ty
            .as_ref()
            .map(TypeSpec::is_nullable)
            .unwrap_or(false);

        let default = default_literal(prop_schema);
        let opt_marker = if nullable || !(is_required || default.is_some()) {
            "?"
        } else {
            ""
        };
        let assignment = default.map(|d| format!(" = {d}")).unwrap_or_default();

        out.push_str(&format!("    {prop_name}{opt_marker}: {ty}{assignment}\n"));

        if let Some(desc) = prop_schema.description.as_deref() {
            out.push_str(&format_docstring(desc, 4));
            out.push('\n');
        }
        Ok(())
    }

    /// Decide the KCL type for a field. Nested objects emit a new
    /// schema and return its name; primitives return the built-in
    /// type; arrays recurse on the element type.
    fn render_type(
        &mut self,
        nested_name: &str,
        schema: &JsonSchema,
    ) -> Result<String, ValuesSchemaError> {
        // Multi-member unions (more than one non-null type) map to a
        // KCL union annotation `T1 | T2`. A single-member union (the
        // common `["string","null"]` optionality idiom) drops through
        // to the primary fast path below — optionality is handled by
        // the field's `?` marker, not the type.
        if let Some(TypeSpec::Union(members)) = schema.ty.as_ref() {
            let non_null: Vec<&str> = members
                .iter()
                .map(String::as_str)
                .filter(|t| *t != "null")
                .collect();
            if non_null.len() > 1 {
                // object/array members can't be named per-member here
                // (we'd need a synthesized schema per branch); fall back
                // to `any` for the whole field rather than guess.
                if non_null.iter().any(|t| *t == "object" || *t == "array") {
                    return Ok("any".to_string());
                }
                let parts: Vec<&str> = non_null.into_iter().map(primitive_kcl_type).collect();
                return Ok(parts.join(" | "));
            }
        }
        let primary = schema.ty.as_ref().and_then(TypeSpec::primary).unwrap_or("");
        Ok(match primary {
            "object" => {
                // Nested object — emit a support schema.
                self.emit_object(schema, nested_name)?;
                nested_name.to_string()
            }
            "array" => {
                let item_ty = match schema.items.as_deref() {
                    Some(inner) => {
                        let inner_name = format!("{nested_name}Item");
                        self.render_type(&inner_name, inner)?
                    }
                    None => "any".to_string(),
                };
                format!("[{item_ty}]")
            }
            other => primitive_kcl_type(other).to_string(),
        })
    }

    fn finish(self) -> String {
        self.out.join("\n")
    }
}

/// Map a JSON Schema primitive name to its KCL built-in. Unknown or
/// unmappable types (`null`, anything we don't model) collapse to `any`.
fn primitive_kcl_type(json_type: &str) -> &'static str {
    match json_type {
        "string" => "str",
        "integer" => "int",
        "number" => "float",
        "boolean" => "bool",
        _ => "any",
    }
}

/// Render a JSON default value as a KCL literal. Returns `None`
/// for shapes we can't render (arbitrary nested dicts, etc.) —
/// the caller falls back to an unpopulated optional field.
fn default_literal(schema: &JsonSchema) -> Option<String> {
    let v = schema.default.as_ref()?;
    match schema.ty.as_ref() {
        Some(ty) => ty.find_map_member(|member| json_value_to_kcl_for_type(v, member)),
        None => json_value_to_kcl(v),
    }
}

fn json_value_to_kcl_for_type(v: &serde_json::Value, json_type: &str) -> Option<String> {
    match json_type {
        "null" => None,
        "boolean" if v.is_boolean() => json_value_to_kcl(v),
        "string" if v.is_string() => json_value_to_kcl(v),
        "integer" => integer_value_to_kcl(v),
        "number" if v.is_number() => json_value_to_kcl(v),
        // Nested defaults require recursive validation against
        // `items` / `properties`; omit them until we model that.
        "array" | "object" => None,
        // Unknown JSON Schema types collapse to KCL `any`.
        _ if !matches!(
            json_type,
            "null" | "boolean" | "string" | "integer" | "number" | "array" | "object"
        ) =>
        {
            json_value_to_kcl(v)
        }
        _ => None,
    }
}

fn integer_value_to_kcl(v: &serde_json::Value) -> Option<String> {
    if let Some(i) = v.as_i64() {
        return Some(i.to_string());
    }
    if let Some(u) = v.as_u64() {
        return Some(u.to_string());
    }
    let f = v.as_f64()?;
    if f.is_finite() && f.fract() == 0.0 {
        return Some(format!("{f:.0}"));
    }
    None
}

fn json_value_to_kcl(v: &serde_json::Value) -> Option<String> {
    Some(match v {
        serde_json::Value::Null => "None".to_string(),
        serde_json::Value::Bool(b) => if *b { "True" } else { "False" }.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => kcl_string_literal(s),
        serde_json::Value::Array(items) => {
            let parts: Option<Vec<String>> = items.iter().map(json_value_to_kcl).collect();
            format!("[{}]", parts?.join(", "))
        }
        serde_json::Value::Object(map) => {
            let mut entries = Vec::with_capacity(map.len());
            for (k, val) in map {
                let rendered = json_value_to_kcl(val)?;
                entries.push(format!("{}: {rendered}", kcl_string_literal(k)));
            }
            format!("{{{}}}", entries.join(", "))
        }
    })
}

/// Format as a KCL string literal — quote + escape `\` and `"`.
/// Pub for reuse by `stdlib::build_chart_module`, which also emits
/// string literals (chart paths) and needs the same escaping rules.
pub(crate) fn kcl_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// True when `s` is a legal KCL identifier (`[A-Za-z_][A-Za-z0-9_]*`).
/// Property names are emitted verbatim into a `schema` body, so only
/// identifier-shaped keys are safe to emit; anything else could carry
/// a statement-injecting payload (newline, `:`, `=`, …) into the
/// consumer's signed render output.
fn is_kcl_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Indent a docstring block at `indent` spaces. KCL docstrings use
/// triple-quoted strings on the line below the field.
///
/// The description text comes from the chart's `values.schema.json` and
/// is untrusted: an embedded `"""` would close the docstring early and
/// let trailing text become schema/module body. We neutralize every
/// run of quotes that could form (or extend) a `"""` delimiter by
/// inserting a backslash, which KCL reads as an escaped quote inside
/// the string rather than a delimiter. This keeps the prose readable
/// while making breakout impossible.
fn format_docstring(text: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let trimmed = neutralize_triple_quotes(text.trim());
    // Single-line doc → single-line docstring, multi-line → block.
    if !trimmed.contains('\n') {
        format!("{pad}\"\"\"{trimmed}\"\"\"")
    } else {
        let body = trimmed
            .lines()
            .map(|l| format!("{pad}{l}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("{pad}\"\"\"\n{body}\n{pad}\"\"\"")
    }
}

/// Escape every `"` in a run of two or more consecutive quotes so the
/// text can never form a `"""` docstring delimiter, and escape a quote
/// adjacent to either end (a leading/trailing quote would merge with
/// the wrapping delimiter into `""""`). A lone interior quote is left
/// as-is — KCL permits a single `"` inside a triple-quoted string.
fn neutralize_triple_quotes(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == '"' {
            let prev_quote = i > 0 && chars[i - 1] == '"';
            let next_quote = i + 1 < chars.len() && chars[i + 1] == '"';
            let at_edge = i == 0 || i + 1 == chars.len();
            // Escape if this quote is part of a >=2 quote run, or sits
            // against the delimiter we're about to wrap it in.
            if prev_quote || next_quote || at_edge {
                out.push('\\');
            }
        }
        out.push(c);
    }
    out
}

/// `replicaCount` / `image_pull_policy` → `ReplicaCount` / `ImagePullPolicy`.
/// Used to name nested schemas off property names. Non-ident chars
/// (dots, slashes from JSON-Pointer-ish keys) are dropped so the
/// result is always a valid KCL identifier. A leading digit or empty
/// result gets `_` prefixed so the identifier parses; no silent empty
/// schema names.
fn pascal_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut capitalize_next = true;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            if capitalize_next {
                out.extend(c.to_uppercase());
                capitalize_next = false;
            } else {
                out.push(c);
            }
        } else {
            // `_`, `-`, `.`, `/`, etc. — treat all as word separators.
            capitalize_next = true;
        }
    }
    // KCL identifiers can't start with a digit and can't be empty.
    match out.chars().next() {
        None => "_".to_string(),
        Some(c) if c.is_ascii_digit() => format!("_{out}"),
        _ => out,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_object_root_produces_empty_output() {
        let input = br#"{ "type": "string" }"#;
        let out = generate_from_bytes(input).unwrap();
        assert_eq!(out.source, "");
    }

    #[test]
    fn primitive_fields_render() {
        let input = br#"{
            "type": "object",
            "properties": {
                "replicaCount": { "type": "integer", "default": 1 },
                "name":         { "type": "string",  "default": "hello" },
                "debug":        { "type": "boolean", "default": false },
                "ratio":        { "type": "number",  "default": 0.5 }
            }
        }"#;
        let out = generate_from_bytes(input).unwrap();
        assert!(
            out.source.contains("replicaCount: int = 1"),
            "{}",
            out.source
        );
        assert!(
            out.source.contains("name: str = \"hello\""),
            "{}",
            out.source
        );
        assert!(out.source.contains("debug: bool = False"), "{}", out.source);
        assert!(out.source.contains("ratio: float = 0.5"), "{}", out.source);
        assert!(out.source.starts_with("schema Values:"));
    }

    #[test]
    fn required_fields_have_no_question_mark() {
        let input = br#"{
            "type": "object",
            "properties": {
                "host":    { "type": "string" },
                "replicas":{ "type": "integer", "default": 2 }
            },
            "required": ["host"]
        }"#;
        let out = generate_from_bytes(input).unwrap();
        assert!(out.source.contains("host: str"), "{}", out.source);
        // Required fields don't get the `?`; neither do fields with
        // a default (they resolve without input).
        assert!(!out.source.contains("host?:"), "{}", out.source);
        assert!(!out.source.contains("replicas?:"), "{}", out.source);
    }

    #[test]
    fn optional_without_default_has_question_mark() {
        let input = br#"{
            "type": "object",
            "properties": {
                "note": { "type": "string" }
            }
        }"#;
        let out = generate_from_bytes(input).unwrap();
        assert!(out.source.contains("note?: str"), "{}", out.source);
    }

    #[test]
    fn descriptions_generate_parseable_kcl_docstrings() {
        let input = br#"{
            "type": "object",
            "description": "Chart values.",
            "properties": {
                "host": {
                    "type": "string",
                    "description": "Public hostname."
                }
            },
            "required": ["host"]
        }"#;

        let out = generate_from_bytes(input).unwrap();
        let issues = crate::package_k::lint_kcl_source("values.k", &out.source).unwrap();

        assert!(
            issues.is_empty(),
            "source:\n{}\nissues: {:?}",
            out.source,
            issues
        );
    }

    #[test]
    fn nested_object_generates_support_schema() {
        let input = br#"{
            "type": "object",
            "properties": {
                "image": {
                    "type": "object",
                    "properties": {
                        "repository": { "type": "string" },
                        "tag":        { "type": "string", "default": "latest" }
                    },
                    "required": ["repository"]
                }
            }
        }"#;
        let out = generate_from_bytes(input).unwrap();
        assert!(out.source.contains("image?: ValuesImage"), "{}", out.source);
        assert!(out.source.contains("schema ValuesImage:"), "{}", out.source);
        assert!(out.source.contains("repository: str"), "{}", out.source);
        // `tag` is optional in JSON Schema but carries a default, so
        // the KCL field resolves without input — no `?` needed.
        assert!(
            out.source.contains("tag: str = \"latest\""),
            "{}",
            out.source
        );
    }

    #[test]
    fn arrays_render_with_item_type() {
        let input = br#"{
            "type": "object",
            "properties": {
                "hosts": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            }
        }"#;
        let out = generate_from_bytes(input).unwrap();
        assert!(out.source.contains("hosts?: [str]"), "{}", out.source);
    }

    #[test]
    fn array_without_items_is_any() {
        let input = br#"{
            "type": "object",
            "properties": {
                "stuff": { "type": "array" }
            }
        }"#;
        let out = generate_from_bytes(input).unwrap();
        assert!(out.source.contains("stuff?: [any]"), "{}", out.source);
    }

    #[test]
    fn nullable_single_member_union_stays_optional() {
        // `["string", "null"]` has one non-null member: it renders as
        // a plain `str` (no `|`), forced optional by the `"null"`.
        let input = br#"{
            "type": "object",
            "properties": {
                "maybe": { "type": ["string", "null"] }
            }
        }"#;
        let out = generate_from_bytes(input).unwrap();
        assert!(out.source.contains("maybe?: str"), "{}", out.source);
    }

    #[test]
    fn multi_member_nullable_union_emits_union_annotation() {
        // A union like `["string","integer","null"]` with a default
        // whose JSON type is only ONE of the members must emit a real
        // KCL union annotation (`int | str`), not collapse to the first
        // member with a contradictory default. Collapsing produced e.g.
        // `port: str = 8080` which aborts the evaluator at type-pack.
        let input = br#"{
            "type": "object",
            "properties": {
                "port": { "type": ["string", "integer", "null"], "default": 8080 }
            }
        }"#;
        let out = generate_from_bytes(input).unwrap();
        // Annotation is a union containing int and str, field optional
        // because the union is nullable.
        assert!(
            out.source.contains("port?: int | str = 8080")
                || out.source.contains("port?: str | int = 8080"),
            "expected union annotation, got:\n{}",
            out.source
        );
        // Must NOT emit the contradictory bare-`str`-with-int-default.
        assert!(
            !out.source.contains("port?: str = 8080") && !out.source.contains("port: str = 8080"),
            "emitted contradictory bare annotation:\n{}",
            out.source
        );
    }

    #[test]
    fn contradictory_defaults_are_not_emitted() {
        // Chart schemas in the wild can declare defaults that do not
        // match their own type. Emitting `field: str = None` hands KCL
        // a schema/default contradiction and can abort the evaluator at
        // type-pack time during render.
        let input = br#"{
            "type": "object",
            "properties": {
                "nullableByDefault": { "type": "string", "default": null },
                "nullableUnion": { "type": ["string", "null"], "default": null },
                "wrongPrimitive": { "type": "string", "default": 80 },
                "wrongArrayItem": {
                    "type": "array",
                    "items": { "type": "string" },
                    "default": [80]
                },
                "floatInteger": { "type": "integer", "default": 1.0 }
            }
        }"#;
        let out = generate_from_bytes(input).unwrap();
        assert!(
            out.source.contains("nullableByDefault?: str"),
            "{}",
            out.source
        );
        assert!(out.source.contains("nullableUnion?: str"), "{}", out.source);
        assert!(
            out.source.contains("wrongPrimitive?: str"),
            "{}",
            out.source
        );
        assert!(
            out.source.contains("wrongArrayItem?: [str]"),
            "{}",
            out.source
        );
        assert!(
            out.source.contains("floatInteger: int = 1"),
            "{}",
            out.source
        );
        assert!(
            !out.source.contains("nullableByDefault: str = None")
                && !out.source.contains("nullableByDefault?: str = None"),
            "emitted contradictory null default:\n{}",
            out.source
        );
        assert!(
            !out.source.contains("nullableUnion?: str = None"),
            "emitted nullable-union null default:\n{}",
            out.source
        );
        assert!(
            !out.source.contains("wrongPrimitive: str = 80")
                && !out.source.contains("wrongPrimitive?: str = 80"),
            "emitted contradictory primitive default:\n{}",
            out.source
        );
        assert!(
            !out.source.contains("wrongArrayItem?: [str] = [80]"),
            "emitted shallow-validated array default:\n{}",
            out.source
        );
    }

    #[test]
    fn non_nullable_union_is_required_union() {
        // `["string","integer"]` (no null) → `str | int`. Because the
        // union is not nullable, the field's `?` is governed by the
        // normal required/default rules — here it's `required`, so no `?`.
        let input = br#"{
            "type": "object",
            "properties": {
                "value": { "type": ["string", "integer"] }
            },
            "required": ["value"]
        }"#;
        let out = generate_from_bytes(input).unwrap();
        assert!(
            out.source.contains("value: str | int"),
            "expected required union, got:\n{}",
            out.source
        );
        assert!(
            !out.source.contains("value?:"),
            "non-nullable union must not be optional:\n{}",
            out.source
        );
    }

    #[test]
    fn pascal_case_handles_snake_and_kebab() {
        assert_eq!(pascal_case("replicaCount"), "ReplicaCount");
        assert_eq!(pascal_case("image_pull_policy"), "ImagePullPolicy");
        assert_eq!(pascal_case("node-selector"), "NodeSelector");
    }

    #[test]
    fn pascal_case_guards_kcl_identifier_shape() {
        // Leading digit → prefixed `_` so the result is a legal KCL
        // identifier. Without the guard the generated `schema 2xx:`
        // would fail to parse.
        assert_eq!(pascal_case("2xx"), "_2xx");
        // Non-ident chars (dots from JSON-Pointer-ish keys) become
        // word boundaries, not identifier content.
        assert_eq!(pascal_case("foo.bar"), "FooBar");
        assert_eq!(pascal_case("api/v1"), "ApiV1");
        // Empty input → sentinel rather than an empty identifier.
        assert_eq!(pascal_case(""), "_");
    }

    #[test]
    fn kcl_string_literal_escapes() {
        assert_eq!(kcl_string_literal("plain"), r#""plain""#);
        assert_eq!(kcl_string_literal(r#"a"b"#), r#""a\"b""#);
        assert_eq!(kcl_string_literal(r"a\b"), r#""a\\b""#);
    }

    #[test]
    fn empty_object_schema_gets_passthrough_field() {
        // Empty objects are legal JSON Schema but KCL schemas need a
        // body. Ensure we emit the placeholder field without crashing.
        let input = br#"{ "type": "object", "properties": {} }"#;
        let out = generate_from_bytes(input).unwrap();
        assert!(out.source.contains("schema Values:"), "{}", out.source);
        assert!(out.source.contains("_: any = None"), "{}", out.source);
    }

    #[test]
    fn malformed_json_surfaces_parse_error() {
        let input = b"not json {{{";
        let err = generate_from_bytes(input).unwrap_err();
        assert!(matches!(err, ValuesSchemaError::Parse(_)));
    }

    #[test]
    fn injected_property_name_is_rejected_not_emitted() {
        // A property key crafted to break out of the field line and
        // inject a top-level statement into `schema Values`. The key
        // contains a `:` + newline + a fabricated assignment.
        let input = br#"{
            "type": "object",
            "properties": {
                "x: int\n    INJECTED = 999\n    y": { "type": "string" }
            }
        }"#;
        let result = generate_from_bytes(input);
        // Either the generator refuses the whole schema (structured
        // error) or it never emits the injected statement. We require
        // the injected token to be absent from any generated source.
        match result {
            Err(ValuesSchemaError::InvalidPropertyName { .. }) => {}
            Ok(gen) => assert!(
                !gen.source.contains("INJECTED"),
                "injected statement leaked into generated source:\n{}",
                gen.source
            ),
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn description_triple_quote_cannot_escape_docstring() {
        // A description that closes the docstring early and appends a
        // fabricated top-level statement.
        let input = br#"{
            "type": "object",
            "description": "ok \"\"\"\n_INJECTED = 1\nschema Evil:\n    z: int\n\"\"\"",
            "properties": {
                "host": { "type": "string" }
            },
            "required": ["host"]
        }"#;
        let out = generate_from_bytes(input).unwrap();
        // The raw `"""` must not survive: any embedded triple-quote is
        // neutralized (escaped) so it cannot close the docstring and let
        // trailing text become module body. No unescaped `"""` should
        // appear inside the description payload.
        assert!(
            !out.source.contains("ok \"\"\""),
            "unescaped triple-quote survived in description:\n{}",
            out.source
        );
        // The injected statement must NOT appear as a top-level (column
        // 0) statement — it stays indented inside the docstring block.
        assert!(
            !out.source.lines().any(|l| l.starts_with("_INJECTED")),
            "injected statement reached module body:\n{}",
            out.source
        );
        assert!(
            !out.source.lines().any(|l| l.starts_with("schema Evil")),
            "injected schema reached module body:\n{}",
            out.source
        );
        // And the generated module must still be valid, parseable KCL —
        // a successful breakout would instead produce a parse error.
        let issues = crate::package_k::lint_kcl_source("values.k", &out.source).unwrap();
        assert!(
            issues.is_empty(),
            "source:\n{}\nissues: {:?}",
            out.source,
            issues
        );
    }

    #[test]
    fn is_kcl_identifier_matches_grammar() {
        assert!(is_kcl_identifier("replicaCount"));
        assert!(is_kcl_identifier("image_pull_policy"));
        assert!(is_kcl_identifier("_private"));
        assert!(is_kcl_identifier("a1"));
        assert!(!is_kcl_identifier(""));
        assert!(!is_kcl_identifier("1abc"));
        assert!(!is_kcl_identifier("node-selector"));
        assert!(!is_kcl_identifier("foo.bar"));
        assert!(!is_kcl_identifier("x: int\n    INJECTED = 1"));
    }
}
