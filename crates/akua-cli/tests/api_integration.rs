//! End-to-end tests for `akua api`.
//!
//! These drive the compiled binary against a local mock server so the
//! hosted API bridge never talks to the real Akua API during tests.

use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

use httpmock::prelude::*;
use httpmock::Method::HEAD;
use serde_json::json;

const AKUA_BIN: &str = env!("CARGO_BIN_EXE_akua");

fn run(cwd: &Path, args: &[&str]) -> Output {
    Command::new(AKUA_BIN)
        .current_dir(cwd)
        .env("AKUA_NO_AGENT_DETECT", "1")
        .args(args)
        .output()
        .expect("spawn akua binary")
}

fn assert_exit(output: &Output, expected: i32) {
    let code = output.status.code().unwrap_or(-1);
    assert_eq!(
        code,
        expected,
        "expected exit {expected}, got {code}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn stdout_json(output: &Output) -> serde_json::Value {
    let text = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(text.trim())
        .unwrap_or_else(|err| panic!("stdout is not JSON: {err}\n--- stdout ---\n{text}"))
}

fn stderr_json(output: &Output) -> serde_json::Value {
    let text = String::from_utf8_lossy(&output.stderr);
    serde_json::from_str(text.trim())
        .unwrap_or_else(|err| panic!("stderr is not JSON: {err}\n--- stderr ---\n{text}"))
}

fn tempdir() -> tempfile::TempDir {
    tempfile::TempDir::new().expect("tempdir")
}

fn api_error(code: u64, message: &str) -> serde_json::Value {
    json!({
        "success": false,
        "errors": [{
            "code": code,
            "message": message,
            "path": ["params", "id"],
            "metadata": { "workspace_id": "ws_missing" }
        }],
        "result": {}
    })
}

#[test]
fn api_get_sends_bearer_token_and_preserves_success_body() {
    let dir = tempdir();
    let server = MockServer::start();
    let base_url = server.url("/v1/");
    let body = json!({
        "success": true,
        "result": [{ "id": "ws_123", "name": "Demo" }]
    });
    let mock = server.mock(|when, then| {
        when.method(GET)
            .path("/v1/workspaces")
            .header("authorization", "Bearer test-token");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(body.clone());
    });

    let out = run(
        dir.path(),
        &[
            "api",
            "--json",
            "--base-url",
            &base_url,
            "--token",
            "test-token",
            "/workspaces",
        ],
    );

    assert_exit(&out, 0);
    mock.assert();
    assert_eq!(stdout_json(&out), body);
}

#[test]
fn api_spec_fetches_public_openapi_document() {
    let dir = tempdir();
    let server = MockServer::start();
    let base_url = server.url("/v1/");
    let body = json!({
        "openapi": "3.1.0",
        "info": { "title": "Akua API", "version": "test" },
        "paths": {}
    });
    let mock = server.mock(|when, then| {
        when.method(GET)
            .path("/v1/openapi.json")
            .header("authorization", "Bearer test-token");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(body.clone());
    });

    let out = run(
        dir.path(),
        &[
            "api",
            "--json",
            "spec",
            "--base-url",
            &base_url,
            "--token",
            "test-token",
        ],
    );

    assert_exit(&out, 0);
    mock.assert();
    assert_eq!(stdout_json(&out), body);
}

#[test]
fn api_spec_public_audience_fetches_openapi_document() {
    let dir = tempdir();
    let server = MockServer::start();
    let base_url = server.url("/v1/");
    let body = json!({
        "openapi": "3.1.0",
        "info": { "title": "Akua API", "version": "test" },
        "paths": {}
    });
    let mock = server.mock(|when, then| {
        when.method(GET)
            .path("/v1/openapi.json")
            .header("authorization", "Bearer test-token");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(body.clone());
    });

    let out = run(
        dir.path(),
        &[
            "api",
            "--json",
            "spec",
            "--audience",
            "public",
            "--base-url",
            &base_url,
            "--token",
            "test-token",
        ],
    );

    assert_exit(&out, 0);
    mock.assert();
    assert_eq!(stdout_json(&out), body);
}

#[test]
fn api_spec_elevated_audiences_are_unsupported_without_fetching() {
    for audience in ["partner", "admin", "internal"] {
        let dir = tempdir();
        let server = MockServer::start();
        let base_url = server.url("/v1/");
        let mock = server.mock(|when, then| {
            when.method(GET).path("/v1/openapi.json");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(json!({ "openapi": "3.1.0" }));
        });

        let out = run(
            dir.path(),
            &[
                "api",
                "--json",
                "spec",
                "--audience",
                audience,
                "--base-url",
                &base_url,
                "--token",
                "test-token",
            ],
        );

        assert_exit(&out, 1);
        assert!(out.stdout.is_empty(), "stdout should be empty");
        let err = stderr_json(&out);
        assert_eq!(err["code"], "E_UNSUPPORTED");
        mock.assert_hits(0);
    }
}

#[test]
fn api_post_sends_fields_workspace_and_idempotency_headers() {
    let dir = tempdir();
    let server = MockServer::start();
    let base_url = server.url("/v1/");
    let body = json!({
        "success": true,
        "result": { "allowed": true }
    });
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/access_decisions")
            .header("authorization", "Bearer test-token")
            .header("akua-context", "ws_123")
            .header("idempotency-key", "idem-123")
            .json_body(json!({ "permission": "offers.create" }));
        then.status(200)
            .header("content-type", "application/json")
            .json_body(body.clone());
    });

    let out = run(
        dir.path(),
        &[
            "api",
            "--json",
            "--idempotency-key",
            "idem-123",
            "--base-url",
            &base_url,
            "--token",
            "test-token",
            "--workspace",
            "ws_123",
            "/access_decisions",
            "-F",
            "permission=offers.create",
        ],
    );

    assert_exit(&out, 0);
    mock.assert();
    assert_eq!(stdout_json(&out), body);
}

#[test]
fn api_401_platform_envelope_maps_to_auth_invalid_user_error() {
    let dir = tempdir();
    let server = MockServer::start();
    let base_url = server.url("/v1/");
    server.mock(|when, then| {
        when.method(GET).path("/v1/workspaces");
        then.status(401)
            .header("content-type", "application/json")
            .json_body(api_error(7003, "token is invalid or expired"));
    });

    let out = run(
        dir.path(),
        &[
            "api",
            "--json",
            "--base-url",
            &base_url,
            "--token",
            "test-token",
            "/workspaces",
        ],
    );

    assert_exit(&out, 1);
    let err = stderr_json(&out);
    assert_eq!(err["code"], "E_AUTH_INVALID");
    assert_eq!(err["message"], "token is invalid or expired");
    assert_eq!(err["field"], "params.id");
}

#[test]
fn api_403_platform_envelope_maps_to_forbidden_user_error() {
    let dir = tempdir();
    let server = MockServer::start();
    let base_url = server.url("/v1/");
    server.mock(|when, then| {
        when.method(POST).path("/v1/access_decisions");
        then.status(403)
            .header("content-type", "application/json")
            .json_body(api_error(7004, "permission denied"));
    });

    let out = run(
        dir.path(),
        &[
            "api",
            "--json",
            "--base-url",
            &base_url,
            "--token",
            "test-token",
            "/access_decisions",
            "-F",
            "permission=offers.create",
        ],
    );

    assert_exit(&out, 1);
    let err = stderr_json(&out);
    assert_eq!(err["code"], "E_FORBIDDEN");
    assert_eq!(err["message"], "permission denied");
    assert_eq!(err["field"], "params.id");
}

#[test]
fn api_429_exits_rate_limited() {
    let dir = tempdir();
    let server = MockServer::start();
    let base_url = server.url("/v1/");
    server.mock(|when, then| {
        when.method(GET).path("/v1/workspaces");
        then.status(429)
            .header("content-type", "application/json")
            .json_body(api_error(7008, "too many requests"));
    });

    let out = run(
        dir.path(),
        &[
            "api",
            "--json",
            "--base-url",
            &base_url,
            "--token",
            "test-token",
            "/workspaces",
        ],
    );

    assert_exit(&out, 4);
    let err = stderr_json(&out);
    assert_eq!(err["message"], "too many requests");
}

#[test]
fn api_empty_success_response_preserves_empty_stdout() {
    let dir = tempdir();
    let server = MockServer::start();
    let base_url = server.url("/v1/");
    server.mock(|when, then| {
        when.method(HEAD).path("/v1/workspaces");
        then.status(204);
    });

    let out = run(
        dir.path(),
        &[
            "api",
            "--json",
            "--base-url",
            &base_url,
            "--token",
            "test-token",
            "-X",
            "HEAD",
            "/workspaces",
        ],
    );

    assert_exit(&out, 0);
    assert!(out.stdout.is_empty(), "stdout should be empty");
}

#[test]
fn api_timeout_exits_timeout() {
    let dir = tempdir();
    let server = MockServer::start();
    let base_url = server.url("/v1/");
    server.mock(|when, then| {
        when.method(GET).path("/v1/workspaces");
        then.status(200)
            .delay(Duration::from_millis(300))
            .header("content-type", "application/json")
            .json_body(json!({ "success": true, "result": [] }));
    });

    let out = run(
        dir.path(),
        &[
            "api",
            "--json",
            "--base-url",
            &base_url,
            "--token",
            "test-token",
            "--timeout",
            "10ms",
            "/workspaces",
        ],
    );

    assert_exit(&out, 6);
    let err = stderr_json(&out);
    assert_eq!(err["code"], "E_API_REQUEST");
}
