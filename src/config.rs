use std::{env, fmt, sync::Arc, time::Duration};

use thiserror::Error;

/// A secret that redacts itself in `Debug` and `Display`.
#[derive(Clone)]
pub struct Secret(Arc<str>);

impl Secret {
    pub fn new(value: impl AsRef<str>) -> Self {
        Self(Arc::from(value.as_ref()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret([redacted])")
    }
}

#[derive(Debug, Error)]
pub enum StartupError {
    #[error("missing {0}. Set it in the environment (see .env.example)")]
    Missing(&'static str),
    #[error("{name} must be at least {min} characters")]
    TooShort { name: &'static str, min: usize },
    #[error("invalid {name}: {detail}")]
    Invalid { name: &'static str, detail: String },
}

#[derive(Clone)]
pub struct Config {
    pub port: u16,
    pub mcp_auth_token: Secret,
    pub ynab_api_key: Secret,
    pub ynab_plan_id: Option<String>,
    pub ynab_api_base: String,
    pub cache_ttl: Duration,
    pub max_requests_per_hour: u32,
    pub allowed_hosts: Vec<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, StartupError> {
        let port = match env::var("PORT") {
            Ok(value) if !value.trim().is_empty() => {
                value
                    .trim()
                    .parse::<u16>()
                    .map_err(|_| StartupError::Invalid {
                        name: "PORT",
                        detail: "must be a number from 1 to 65535".into(),
                    })?
            }
            _ => 8080,
        };
        if port == 0 {
            return Err(StartupError::Invalid {
                name: "PORT",
                detail: "must be a number from 1 to 65535".into(),
            });
        }
        let mcp_auth_token = require_secret(&["MCP_AUTH_TOKEN", "MCP_BEARER_TOKEN"], 16)?;
        let ynab_api_key = require_secret(&["YNAB_API_KEY", "YNAB_ACCESS_TOKEN"], 8)?;
        let ynab_plan_id = optional_env("YNAB_PLAN_ID");
        if let Some(plan_id) = &ynab_plan_id {
            crate::validate::validate_path_id("YNAB_PLAN_ID", plan_id).map_err(|detail| {
                StartupError::Invalid {
                    name: "YNAB_PLAN_ID",
                    detail,
                }
            })?;
        }
        let ynab_api_base =
            optional_env("YNAB_API_BASE").unwrap_or_else(|| "https://api.ynab.com/v1".into());
        reqwest::Url::parse(&ynab_api_base).map_err(|error| StartupError::Invalid {
            name: "YNAB_API_BASE",
            detail: error.to_string(),
        })?;
        let cache_ttl = Duration::from_secs(parse_u64("YNAB_CACHE_TTL_SECONDS", 30)?);
        let max_requests_per_hour = parse_u64("YNAB_MAX_REQUESTS_PER_HOUR", 180)? as u32;
        let allowed_hosts = optional_env("MCP_ALLOWED_HOSTS")
            .map(|value| {
                value
                    .split(',')
                    .map(|host| host.trim().to_string())
                    .filter(|host| !host.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| {
                vec![
                    "localhost".into(),
                    "127.0.0.1".into(),
                    "::1".into(),
                    "ynab.mazlabs.com".into(),
                ]
            });
        if allowed_hosts.is_empty() {
            return Err(StartupError::Invalid {
                name: "MCP_ALLOWED_HOSTS",
                detail: "must list at least one host, or unset it to use the defaults".into(),
            });
        }
        Ok(Self {
            port,
            mcp_auth_token,
            ynab_api_key,
            ynab_plan_id,
            ynab_api_base,
            cache_ttl,
            max_requests_per_hour,
            allowed_hosts,
        })
    }

    /// Settings for tests. Tokens are obvious fakes and must never be real credentials.
    pub fn for_test(
        ynab_api_base: impl Into<String>,
        auth_token: &str,
        ynab_api_key: &str,
    ) -> Self {
        Self {
            port: 0,
            mcp_auth_token: Secret::new(auth_token),
            ynab_api_key: Secret::new(ynab_api_key),
            ynab_plan_id: None,
            ynab_api_base: ynab_api_base.into(),
            cache_ttl: Duration::ZERO,
            max_requests_per_hour: 0,
            allowed_hosts: vec!["127.0.0.1".into(), "localhost".into()],
        }
    }
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn require_secret(names: &[&'static str], min_len: usize) -> Result<Secret, StartupError> {
    let (name, value) = names
        .iter()
        .find_map(|name| optional_env(name).map(|value| (*name, value)))
        .ok_or(StartupError::Missing(names[0]))?;
    if value
        .chars()
        .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return Err(StartupError::Invalid {
            name,
            detail: "must be a single token without spaces".into(),
        });
    }
    if value.chars().count() < min_len {
        return Err(StartupError::TooShort { name, min: min_len });
    }
    Ok(Secret::new(value))
}

fn parse_u64(name: &'static str, default: u64) -> Result<u64, StartupError> {
    match optional_env(name) {
        None => Ok(default),
        Some(value) => value.parse::<u64>().map_err(|_| StartupError::Invalid {
            name,
            detail: "must be a non-negative integer".into(),
        }),
    }
}
