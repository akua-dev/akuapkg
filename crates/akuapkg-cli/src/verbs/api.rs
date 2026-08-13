use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use akua_core::cli_contract::{codes, ExitCode, StructuredError};
use akua_core::duration_parse::parse_go_duration;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::api_config::{resolve_api_config_from_env, resolve_required_token, ApiConfigInput};
use crate::contract::Context;

#[derive(Debug, Clone)]
pub struct ApiArgs {
    pub path_or_url: String,
    pub method: Option<String>,
    pub headers: Vec<String>,
    pub raw_fields: Vec<String>,
    pub fields: Vec<String>,
    pub input: Option<PathBuf>,
    pub jq: Option<String>,
    pub include: bool,
    pub silent: bool,
    pub paginate: bool,
    pub slurp: bool,
    pub base_url: Option<String>,
    pub token: Option<String>,
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestPlan {
    pub method: reqwest::Method,
    pub url: reqwest::Url,
    pub headers: Vec<(String, String)>,
    pub body: Option<Value>,
}

#[derive(Debug)]
pub struct ApiExecutionError {
    structured: StructuredError,
    exit_code: ExitCode,
}

impl ApiExecutionError {
    fn new(structured: StructuredError, exit_code: ExitCode) -> Self {
        Self {
            structured,
            exit_code,
        }
    }

    pub fn to_structured(&self) -> StructuredError {
        self.structured.clone()
    }

    pub fn exit_code(&self) -> ExitCode {
        self.exit_code
    }
}

pub fn run<W: Write>(
    ctx: &Context,
    args: &ApiArgs,
    stdout: &mut W,
) -> Result<ExitCode, ApiExecutionError> {
    let plan = build_request_plan(ctx, args)
        .map_err(|err| ApiExecutionError::new(err, ExitCode::UserError))?;
    let client = build_client(ctx)?;
    let response = send_request(&client, &plan)?;
    let status = response.status();
    let body = response.text().map_err(map_transport_error)?;

    if status.is_success() {
        if !args.silent {
            stdout
                .write_all(body.as_bytes())
                .map_err(map_stdout_error)?;
            if !body.is_empty() && !body.ends_with('\n') {
                writeln!(stdout).map_err(map_stdout_error)?;
            }
        }
        return Ok(ExitCode::Success);
    }

    Err(ApiExecutionError::new(
        structured_http_error(status, &plan, &body),
        exit_code_for_status(status),
    ))
}

pub fn build_request_plan(ctx: &Context, args: &ApiArgs) -> Result<RequestPlan, StructuredError> {
    reject_deferred_response_processing(args)?;

    let config = resolve_api_config_from_env(ApiConfigInput {
        base_url: args.base_url.clone(),
        token: args.token.clone(),
        workspace: args.workspace.clone(),
    })?;
    let token = resolve_required_token(config.token.as_deref(), !ctx.interactive)?;
    let method = resolve_method(args)?;
    let is_write = is_write_method(&method);
    let mut url = resolve_request_url(&config.base_url, &args.path_or_url)?;
    let mut headers = vec![
        ("accept".to_string(), "application/json".to_string()),
        ("authorization".to_string(), format!("Bearer {token}")),
    ];

    if let Some(workspace) = config
        .workspace
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        headers.push(("akua-context".to_string(), workspace.to_string()));
    }
    if is_write {
        if let Some(key) = ctx
            .idempotency_key
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            headers.push(("idempotency-key".to_string(), key.to_string()));
        }
    }
    for header in &args.headers {
        let (name, value) = parse_header(header)?;
        headers.push((name, value));
    }

    let mut body = args.input.as_deref().map(read_json_input).transpose()?;
    let parsed_fields = parse_fields(args)?;
    if is_read_method(&method) || body.is_some() {
        append_query_fields(&mut url, &parsed_fields);
    } else if !parsed_fields.is_empty() {
        let mut object = Map::new();
        for (key, value) in parsed_fields {
            insert_field(&mut object, &key, value)?;
        }
        body = Some(Value::Object(object));
    }
    if body.is_some() {
        headers.push(("content-type".to_string(), "application/json".to_string()));
    }

    Ok(RequestPlan {
        method,
        url,
        headers,
        body,
    })
}

fn build_client(ctx: &Context) -> Result<reqwest::blocking::Client, ApiExecutionError> {
    let timeout = match ctx.timeout.as_deref() {
        Some(raw) => Some(parse_go_duration(raw).map_err(|reason| {
            ApiExecutionError::new(
                StructuredError::new(
                    codes::E_INVALID_FLAG,
                    format!("--timeout `{raw}`: {reason}"),
                )
                .with_suggestion("--timeout takes a Go-duration string: 30s, 5m, 1h, 250ms.")
                .with_default_docs(),
                ExitCode::UserError,
            )
        })?),
        None => Some(Duration::from_secs(30)),
    };

    reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|source| {
            ApiExecutionError::new(
                StructuredError::new(
                    codes::E_API_REQUEST,
                    format!("failed to build API HTTP client: {source}"),
                )
                .with_default_docs(),
                ExitCode::SystemError,
            )
        })
}

fn send_request(
    client: &reqwest::blocking::Client,
    plan: &RequestPlan,
) -> Result<reqwest::blocking::Response, ApiExecutionError> {
    let mut request = client.request(plan.method.clone(), plan.url.clone());
    for (name, value) in &plan.headers {
        request = request.header(name.as_str(), value.as_str());
    }
    if let Some(body) = &plan.body {
        request = request.json(body);
    }

    request.send().map_err(map_transport_error)
}

fn map_transport_error(source: reqwest::Error) -> ApiExecutionError {
    if source.is_timeout() {
        return ApiExecutionError::new(
            StructuredError::new(
                codes::E_API_REQUEST,
                format!("API request timed out: {source}"),
            )
            .with_suggestion("Pass a larger --timeout value, or retry the request later.")
            .with_default_docs(),
            ExitCode::Timeout,
        );
    }

    let exit_code = if source.is_builder() {
        ExitCode::UserError
    } else {
        ExitCode::SystemError
    };
    let code = if source.is_builder() {
        codes::E_INVALID_FLAG
    } else {
        codes::E_API_REQUEST
    };

    ApiExecutionError::new(
        StructuredError::new(code, format!("API request failed: {source}")).with_default_docs(),
        exit_code,
    )
}

fn map_stdout_error(source: std::io::Error) -> ApiExecutionError {
    ApiExecutionError::new(
        StructuredError::new(
            codes::E_IO,
            format!("failed to write API response to stdout: {source}"),
        )
        .with_default_docs(),
        ExitCode::SystemError,
    )
}

fn structured_http_error(status: StatusCode, plan: &RequestPlan, body: &str) -> StructuredError {
    let code = error_code_for_status(status);
    if let Some(platform) = parse_platform_error(body) {
        let mut structured =
            StructuredError::new(code, platform.message).with_path(plan.url.path().to_string());
        if let Some(field) = platform.field {
            structured = structured.with_field(field);
        }
        return structured.with_default_docs();
    }

    StructuredError::new(
        code,
        format!("API request failed with HTTP status {}", status.as_u16()),
    )
    .with_path(plan.url.path().to_string())
    .with_default_docs()
}

fn parse_platform_error(body: &str) -> Option<ParsedPlatformError> {
    let envelope: PlatformEnvelope = serde_json::from_str(body).ok()?;
    if envelope.success {
        return None;
    }
    let first = envelope.errors.into_iter().next()?;
    Some(ParsedPlatformError {
        message: first.message,
        field: first.path.map(|path| path.join(".")),
    })
}

fn error_code_for_status(status: StatusCode) -> &'static str {
    match status {
        StatusCode::UNAUTHORIZED => codes::E_AUTH_INVALID,
        StatusCode::FORBIDDEN => codes::E_FORBIDDEN,
        _ => codes::E_API_REQUEST,
    }
}

fn exit_code_for_status(status: StatusCode) -> ExitCode {
    match status {
        StatusCode::TOO_MANY_REQUESTS => ExitCode::RateLimited,
        status if status.is_server_error() => ExitCode::SystemError,
        _ => ExitCode::UserError,
    }
}

#[derive(Debug, Deserialize)]
struct PlatformEnvelope {
    success: bool,
    #[serde(default)]
    errors: Vec<PlatformError>,
}

#[derive(Debug, Deserialize)]
struct PlatformError {
    message: String,
    path: Option<Vec<String>>,
}

#[derive(Debug)]
struct ParsedPlatformError {
    message: String,
    field: Option<String>,
}

pub fn parse_field_value(raw: &str) -> Value {
    match raw {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "null" => Value::Null,
        _ => raw
            .parse::<i64>()
            .map(|number| Value::Number(number.into()))
            .unwrap_or_else(|_| Value::String(raw.to_string())),
    }
}

pub fn insert_field(
    object: &mut Map<String, Value>,
    key: &str,
    value: Value,
) -> Result<(), StructuredError> {
    let parts = parse_field_path(key)?;
    insert_field_path(object, &parts, value)
}

fn reject_deferred_response_processing(args: &ApiArgs) -> Result<(), StructuredError> {
    let unsupported = if args.jq.is_some() {
        Some("--jq")
    } else if args.include {
        Some("--include")
    } else if args.paginate {
        Some("--paginate")
    } else if args.slurp {
        Some("--slurp")
    } else {
        None
    };

    if let Some(flag) = unsupported {
        return Err(StructuredError::new(
            codes::E_UNSUPPORTED,
            format!("{flag} is not implemented for `akuapkg api` yet"),
        )
        .with_default_docs());
    }

    Ok(())
}

fn resolve_method(args: &ApiArgs) -> Result<reqwest::Method, StructuredError> {
    let method = args.method.clone().unwrap_or_else(|| {
        if args.input.is_some() || !args.fields.is_empty() || !args.raw_fields.is_empty() {
            "POST".to_string()
        } else {
            "GET".to_string()
        }
    });

    reqwest::Method::from_bytes(method.as_bytes()).map_err(|source| {
        StructuredError::new(
            codes::E_INVALID_FLAG,
            format!("HTTP method `{method}` is invalid: {source}"),
        )
        .with_default_docs()
    })
}

fn is_write_method(method: &reqwest::Method) -> bool {
    !is_read_method(method)
}

fn is_read_method(method: &reqwest::Method) -> bool {
    matches!(*method, reqwest::Method::GET | reqwest::Method::HEAD)
}

fn resolve_request_url(
    base_url: &reqwest::Url,
    path_or_url: &str,
) -> Result<reqwest::Url, StructuredError> {
    if path_or_url.starts_with("http://") || path_or_url.starts_with("https://") {
        let url = reqwest::Url::parse(path_or_url).map_err(|source| {
            StructuredError::new(
                codes::E_INVALID_FLAG,
                format!("API URL `{path_or_url}` is invalid: {source}"),
            )
            .with_default_docs()
        })?;
        if url.origin() != base_url.origin() {
            return Err(StructuredError::new(
                codes::E_INVALID_FLAG,
                "absolute API URL must use the configured API origin",
            )
            .with_default_docs());
        }
        return Ok(url);
    }

    let mut base = base_url.clone();
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path().trim_end_matches('/'));
        base.set_path(&path);
    }

    let relative = path_or_url.trim_start_matches('/');
    let relative = relative.strip_prefix("v1/").unwrap_or(relative);
    base.join(relative).map_err(|source| {
        StructuredError::new(
            codes::E_INVALID_FLAG,
            format!("API path `{path_or_url}` cannot be resolved: {source}"),
        )
        .with_default_docs()
    })
}

fn parse_header(raw: &str) -> Result<(String, String), StructuredError> {
    let (name, value) = raw
        .split_once(':')
        .or_else(|| raw.split_once('='))
        .ok_or_else(|| {
            StructuredError::new(
                codes::E_INVALID_FLAG,
                format!("header `{raw}` must use `name:value` or `name=value`"),
            )
            .with_default_docs()
        })?;

    let name = name.trim().to_ascii_lowercase();
    if name.is_empty() {
        return Err(
            StructuredError::new(codes::E_INVALID_FLAG, "header name is empty").with_default_docs(),
        );
    }

    Ok((name, value.trim().to_string()))
}

fn parse_fields(args: &ApiArgs) -> Result<Vec<(String, Value)>, StructuredError> {
    args.raw_fields
        .iter()
        .map(|field| parse_field_assignment(field, false))
        .chain(
            args.fields
                .iter()
                .map(|field| parse_field_assignment(field, true)),
        )
        .collect()
}

fn parse_field_assignment(raw: &str, typed: bool) -> Result<(String, Value), StructuredError> {
    let (key, value) = raw.split_once('=').ok_or_else(|| {
        StructuredError::new(
            codes::E_INVALID_FLAG,
            format!("field `{raw}` must use `key=value`"),
        )
        .with_default_docs()
    })?;
    if key.is_empty() {
        return Err(
            StructuredError::new(codes::E_INVALID_FLAG, "field name is empty").with_default_docs(),
        );
    }

    let value = if typed {
        parse_field_value(value)
    } else {
        Value::String(value.to_string())
    };

    Ok((key.to_string(), value))
}

fn append_query_fields(url: &mut reqwest::Url, fields: &[(String, Value)]) {
    if fields.is_empty() {
        return;
    }

    let mut query = url.query_pairs_mut();
    for (key, value) in fields {
        query.append_pair(key, &query_value(value));
    }
}

fn query_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Null => "null".to_string(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

fn read_json_input(path: &Path) -> Result<Value, StructuredError> {
    let file = std::fs::File::open(path).map_err(|source| {
        StructuredError::new(
            codes::E_API_REQUEST,
            format!("failed to read --input `{}`: {source}", path.display()),
        )
        .with_default_docs()
    })?;

    serde_json::from_reader(file).map_err(|source| {
        StructuredError::new(
            codes::E_INVALID_FLAG,
            format!("--input `{}` is not valid JSON: {source}", path.display()),
        )
        .with_default_docs()
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FieldPathPart {
    Key(String),
    Array(String),
}

fn parse_field_path(key: &str) -> Result<Vec<FieldPathPart>, StructuredError> {
    if key.is_empty() {
        return Err(
            StructuredError::new(codes::E_INVALID_FLAG, "field name is empty").with_default_docs(),
        );
    }

    let mut parts = Vec::new();
    let mut cursor = key;
    if let Some((head, rest)) = cursor.split_once('[') {
        if head.is_empty() {
            return Err(invalid_field_path(key));
        }
        parts.push(FieldPathPart::Key(head.to_string()));
        cursor = rest;
    } else {
        parts.push(FieldPathPart::Key(cursor.to_string()));
        return Ok(parts);
    }

    loop {
        let (part, rest) = cursor
            .split_once(']')
            .ok_or_else(|| invalid_field_path(key))?;
        if part.is_empty() {
            let FieldPathPart::Key(previous) =
                parts.pop().ok_or_else(|| invalid_field_path(key))?
            else {
                return Err(invalid_field_path(key));
            };
            parts.push(FieldPathPart::Array(previous));
        } else {
            parts.push(FieldPathPart::Key(part.to_string()));
        }

        if rest.is_empty() {
            return Ok(parts);
        }
        cursor = rest
            .strip_prefix('[')
            .ok_or_else(|| invalid_field_path(key))?;
    }
}

fn insert_field_path(
    object: &mut Map<String, Value>,
    parts: &[FieldPathPart],
    value: Value,
) -> Result<(), StructuredError> {
    match parts {
        [FieldPathPart::Key(key)] => {
            object.insert(key.clone(), value);
            Ok(())
        }
        [FieldPathPart::Array(key)] => {
            let slot = object
                .entry(key.clone())
                .or_insert_with(|| Value::Array(Vec::new()));
            match slot {
                Value::Array(values) => {
                    values.push(value);
                    Ok(())
                }
                _ => Err(invalid_field_path(key)),
            }
        }
        [FieldPathPart::Key(key), rest @ ..] => {
            let slot = object
                .entry(key.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            match slot {
                Value::Object(nested) => insert_field_path(nested, rest, value),
                _ => Err(invalid_field_path(key)),
            }
        }
        [FieldPathPart::Array(_), ..] | [] => Err(StructuredError::new(
            codes::E_INVALID_FLAG,
            "array fields cannot contain nested children",
        )
        .with_default_docs()),
    }
}

fn invalid_field_path(path: &str) -> StructuredError {
    StructuredError::new(
        codes::E_INVALID_FLAG,
        format!("field path `{path}` is invalid"),
    )
    .with_default_docs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api_args(path_or_url: &str) -> ApiArgs {
        ApiArgs {
            path_or_url: path_or_url.to_string(),
            method: None,
            headers: Vec::new(),
            raw_fields: Vec::new(),
            fields: Vec::new(),
            input: None,
            jq: None,
            include: false,
            silent: false,
            paginate: false,
            slurp: false,
            base_url: Some("https://api.akua.dev/v1/".to_string()),
            token: Some("test-token".to_string()),
            workspace: None,
        }
    }

    fn plan(args: ApiArgs) -> RequestPlan {
        build_request_plan(&Context::human(), &args).expect("request plan")
    }

    fn header_value<'a>(plan: &'a RequestPlan, name: &str) -> Option<&'a str> {
        plan.headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn parse_field_value_recognizes_json_scalars_and_strings() {
        assert_eq!(parse_field_value("true"), serde_json::json!(true));
        assert_eq!(parse_field_value("false"), serde_json::json!(false));
        assert_eq!(parse_field_value("null"), serde_json::Value::Null);
        assert_eq!(parse_field_value("42"), serde_json::json!(42));
        assert_eq!(parse_field_value("hello"), serde_json::json!("hello"));
    }

    #[test]
    fn insert_field_supports_nested_objects_and_repeated_arrays() {
        let mut body = serde_json::Map::new();

        insert_field(&mut body, "owner[id]", serde_json::json!("w_123")).unwrap();
        insert_field(&mut body, "ids[]", serde_json::json!("a")).unwrap();
        insert_field(&mut body, "ids[]", serde_json::json!("b")).unwrap();

        assert_eq!(
            serde_json::Value::Object(body),
            serde_json::json!({
                "owner": { "id": "w_123" },
                "ids": ["a", "b"]
            })
        );
    }

    #[test]
    fn defaults_to_get_without_fields_or_input() {
        let plan = plan(api_args("/workspaces"));

        assert_eq!(plan.method, reqwest::Method::GET);
        assert_eq!(plan.url.as_str(), "https://api.akua.dev/v1/workspaces");
        assert!(plan.body.is_none());
    }

    #[test]
    fn defaults_to_post_when_fields_are_present() {
        let mut args = api_args("/workspaces");
        args.fields.push("name=demo".to_string());

        let plan = plan(args);

        assert_eq!(plan.method, reqwest::Method::POST);
        assert_eq!(plan.body, Some(serde_json::json!({ "name": "demo" })));
    }

    #[test]
    fn defaults_to_post_when_input_is_present() {
        let tempdir = tempfile::tempdir().unwrap();
        let input = tempdir.path().join("body.json");
        std::fs::write(&input, r#"{"name":"demo"}"#).unwrap();
        let mut args = api_args("/workspaces");
        args.input = Some(input);

        let plan = plan(args);

        assert_eq!(plan.method, reqwest::Method::POST);
        assert_eq!(plan.body, Some(serde_json::json!({ "name": "demo" })));
    }

    #[test]
    fn explicit_get_sends_fields_as_query_parameters() {
        let mut args = api_args("/workspaces");
        args.method = Some("GET".to_string());
        args.fields.push("name=demo".to_string());
        args.fields.push("owner[id]=w_123".to_string());

        let plan = plan(args);

        assert_eq!(plan.method, reqwest::Method::GET);
        assert_eq!(
            plan.url.as_str(),
            "https://api.akua.dev/v1/workspaces?name=demo&owner%5Bid%5D=w_123"
        );
        assert!(plan.body.is_none());
    }

    #[test]
    fn explicit_head_sends_fields_as_query_parameters() {
        let mut args = api_args("/workspaces");
        args.method = Some("HEAD".to_string());
        args.fields.push("name=demo".to_string());

        let plan = plan(args);

        assert_eq!(plan.method, reqwest::Method::HEAD);
        assert_eq!(
            plan.url.as_str(),
            "https://api.akua.dev/v1/workspaces?name=demo"
        );
        assert!(plan.body.is_none());
    }

    #[test]
    fn absolute_urls_must_stay_under_configured_api_origin() {
        let mut args = api_args("https://example.com/v1/workspaces");
        args.token = Some("secret-token".to_string());

        let err = build_request_plan(&Context::human(), &args).unwrap_err();

        assert_eq!(err.code, akua_core::cli_contract::codes::E_INVALID_FLAG);
        assert!(err.message.contains("configured API origin"));
    }

    #[test]
    fn input_body_keeps_field_flags_as_query_parameters() {
        let tempdir = tempfile::tempdir().unwrap();
        let input = tempdir.path().join("body.json");
        std::fs::write(&input, r#"{"displayName":"Demo"}"#).unwrap();
        let mut args = api_args("/products");
        args.input = Some(input);
        args.fields.push("workspaceId=ws_123".to_string());

        let plan = plan(args);

        assert_eq!(plan.method, reqwest::Method::POST);
        assert_eq!(
            plan.url.as_str(),
            "https://api.akua.dev/v1/products?workspaceId=ws_123"
        );
        assert_eq!(
            plan.body,
            Some(serde_json::json!({ "displayName": "Demo" }))
        );
    }

    #[test]
    fn workspace_flag_sends_akua_context_header() {
        let mut args = api_args("/workspaces");
        args.workspace = Some("ws_123".to_string());

        let plan = plan(args);

        assert_eq!(header_value(&plan, "akua-context"), Some("ws_123"));
    }

    #[test]
    fn idempotency_key_is_sent_on_write_requests() {
        let mut args = api_args("/workspaces");
        args.fields.push("name=demo".to_string());
        let ctx = Context {
            idempotency_key: Some("abc".to_string()),
            ..Context::human()
        };

        let plan = build_request_plan(&ctx, &args).expect("request plan");

        assert_eq!(plan.method, reqwest::Method::POST);
        assert_eq!(header_value(&plan, "idempotency-key"), Some("abc"));
    }

    #[test]
    fn version_prefixed_path_normalizes_to_base_version() {
        let plain = plan(api_args("/workspaces"));
        let prefixed = plan(api_args("/v1/workspaces"));

        assert_eq!(plain.url, prefixed.url);
        assert_eq!(plain.url.as_str(), "https://api.akua.dev/v1/workspaces");
    }

    #[test]
    fn unsupported_response_processing_flags_return_unsupported() {
        for args in [
            {
                let mut args = api_args("/workspaces");
                args.jq = Some(".items".to_string());
                args
            },
            {
                let mut args = api_args("/workspaces");
                args.paginate = true;
                args
            },
            {
                let mut args = api_args("/workspaces");
                args.slurp = true;
                args
            },
            {
                let mut args = api_args("/workspaces");
                args.include = true;
                args
            },
        ] {
            let err = build_request_plan(&Context::human(), &args).unwrap_err();
            assert_eq!(err.code, akua_core::cli_contract::codes::E_UNSUPPORTED);
        }
    }
}
