use akua_core::cli_contract::agent::{EnvReader, StdEnvReader};
use akua_core::cli_contract::{codes, StructuredError};

pub const DEFAULT_API_BASE_URL: &str = "https://api.akua.dev/v1/";

const ENV_API_BASE_URL: &str = "AKUA_API_BASE_URL";
const ENV_API_TOKEN: &str = "AKUA_API_TOKEN";
const ENV_WORKSPACE_ID: &str = "AKUA_WORKSPACE_ID";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApiConfigInput {
    pub base_url: Option<String>,
    pub token: Option<String>,
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedApiConfig {
    pub base_url: reqwest::Url,
    pub token: Option<String>,
    pub workspace: Option<String>,
}

pub fn resolve_api_config(
    input: ApiConfigInput,
    env: &impl EnvReader,
) -> Result<ResolvedApiConfig, StructuredError> {
    let env_base_url = env.get(ENV_API_BASE_URL);
    let base_url = match input.base_url.as_deref().or(env_base_url.as_deref()) {
        Some(raw) => parse_base_url(raw)?,
        None => parse_base_url(DEFAULT_API_BASE_URL)?,
    };

    Ok(ResolvedApiConfig {
        base_url,
        token: input.token.or_else(|| env.get(ENV_API_TOKEN)),
        workspace: input.workspace.or_else(|| env.get(ENV_WORKSPACE_ID)),
    })
}

pub fn resolve_api_config_from_env(
    input: ApiConfigInput,
) -> Result<ResolvedApiConfig, StructuredError> {
    resolve_api_config(input, &StdEnvReader)
}

pub fn resolve_required_token<'a>(
    token: Option<&'a str>,
    non_interactive: bool,
) -> Result<&'a str, StructuredError> {
    token
        .and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .ok_or_else(|| {
            let message = if non_interactive {
                "hosted Akua API auth token required; pass --token or set AKUA_API_TOKEN"
            } else {
                "hosted Akua API auth token required"
            };
            StructuredError::new(codes::E_AUTH_REQUIRED, message)
                .with_suggestion("set AKUA_API_TOKEN or pass --token")
                .with_default_docs()
        })
}

fn parse_base_url(raw: &str) -> Result<reqwest::Url, StructuredError> {
    let url = reqwest::Url::parse(raw).map_err(|source| {
        StructuredError::new(
            codes::E_INVALID_FLAG,
            format!("--base-url `{raw}` is not a valid URL: {source}"),
        )
        .with_default_docs()
    })?;

    if !matches!(url.scheme(), "http" | "https") {
        return Err(invalid_base_url(raw, "must start with http:// or https://"));
    }
    if !url.has_host() {
        return Err(invalid_base_url(raw, "must include a host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid_base_url(
            raw,
            "must not include embedded credentials",
        ));
    }

    Ok(url)
}

fn invalid_base_url(raw: &str, reason: &str) -> StructuredError {
    StructuredError::new(
        codes::E_INVALID_FLAG,
        format!("--base-url `{raw}` {reason}"),
    )
    .with_default_docs()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use akua_core::cli_contract::agent::EnvReader;

    use super::*;

    #[derive(Default)]
    struct TestEnv {
        vars: HashMap<String, String>,
    }

    impl TestEnv {
        fn with_api_token(token: &str) -> Self {
            Self::default().with("AKUA_API_TOKEN", token)
        }

        fn with_workspace_id(workspace: &str) -> Self {
            Self::default().with("AKUA_WORKSPACE_ID", workspace)
        }

        fn with(mut self, key: &str, value: &str) -> Self {
            self.vars.insert(key.to_string(), value.to_string());
            self
        }
    }

    impl EnvReader for TestEnv {
        fn get(&self, key: &str) -> Option<String> {
            self.vars.get(key).cloned()
        }
    }

    #[test]
    fn defaults_to_current_akua_public_api_base_url() {
        let env = TestEnv::default();
        let resolved = resolve_api_config(ApiConfigInput::default(), &env).unwrap();

        assert_eq!(resolved.base_url.as_str(), "https://api.akua.dev/v1/");
    }

    #[test]
    fn explicit_token_wins_over_env() {
        let env = TestEnv::with_api_token("env-token");
        let resolved = resolve_api_config(
            ApiConfigInput {
                token: Some("flag-token".into()),
                ..Default::default()
            },
            &env,
        )
        .unwrap();

        assert_eq!(resolved.token.as_deref(), Some("flag-token"));
    }

    #[test]
    fn api_token_env_is_used_for_hosted_api_auth() {
        let env = TestEnv::with_api_token("env-token");
        let resolved = resolve_api_config(ApiConfigInput::default(), &env).unwrap();

        assert_eq!(resolved.token.as_deref(), Some("env-token"));
    }

    #[test]
    fn workspace_id_env_becomes_workspace_context() {
        let env = TestEnv::with_workspace_id("ws_j572abc123def456");
        let resolved = resolve_api_config(ApiConfigInput::default(), &env).unwrap();

        assert_eq!(resolved.workspace.as_deref(), Some("ws_j572abc123def456"));
    }

    #[test]
    fn non_interactive_missing_token_returns_auth_required() {
        let err = resolve_required_token(None, true).unwrap_err();

        assert_eq!(err.code, "E_AUTH_REQUIRED");
    }

    #[test]
    fn whitespace_only_token_is_auth_required() {
        let err = resolve_required_token(Some(" \t\n"), true).unwrap_err();

        assert_eq!(err.code, "E_AUTH_REQUIRED");
    }

    #[test]
    fn invalid_base_url_is_rejected() {
        let env = TestEnv::default();
        let err = resolve_api_config(
            ApiConfigInput {
                base_url: Some("not a url".into()),
                ..Default::default()
            },
            &env,
        )
        .unwrap_err();

        assert_eq!(err.code, "E_INVALID_FLAG");
    }

    #[test]
    fn base_url_must_be_http_or_https() {
        let env = TestEnv::default();
        let err = resolve_api_config(
            ApiConfigInput {
                base_url: Some("file:///tmp/openapi.json".into()),
                ..Default::default()
            },
            &env,
        )
        .unwrap_err();

        assert_eq!(err.code, "E_INVALID_FLAG");
        assert!(err.message.contains("http:// or https://"));
    }

    #[test]
    fn base_url_must_have_host() {
        let env = TestEnv::default();
        let err = resolve_api_config(
            ApiConfigInput {
                base_url: Some("http:///".into()),
                ..Default::default()
            },
            &env,
        )
        .unwrap_err();

        assert_eq!(err.code, "E_INVALID_FLAG");
        assert!(err.message.contains("host"));
    }

    #[test]
    fn base_url_rejects_embedded_credentials() {
        let env = TestEnv::default();
        let err = resolve_api_config(
            ApiConfigInput {
                base_url: Some("https://user:pass@api.akua.dev/v1/".into()),
                ..Default::default()
            },
            &env,
        )
        .unwrap_err();

        assert_eq!(err.code, "E_INVALID_FLAG");
        assert!(err.message.contains("credentials"));
    }

    #[test]
    fn legacy_env_names_are_ignored() {
        let env = TestEnv::default()
            .with("AKUA_TOKEN", "legacy-token")
            .with("AKUA_BASE_URL", "https://legacy.example/v1/")
            .with("AKUA_WORKSPACE", "legacy-workspace");
        let resolved = resolve_api_config(ApiConfigInput::default(), &env).unwrap();

        assert_eq!(resolved.base_url.as_str(), "https://api.akua.dev/v1/");
        assert!(resolved.token.is_none());
        assert!(resolved.workspace.is_none());
    }
}
