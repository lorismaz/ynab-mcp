//! Thin client for `https://api.ynab.com/v1`.
//!
//! GET responses are cached briefly. Mutations clear the cache. A local
//! hourly cap stays under YNAB's roughly 200 requests per hour.

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::{Duration, Instant},
};

use reqwest::StatusCode;
use serde_json::Value;
use thiserror::Error;

use crate::config::{Config, Secret};

#[derive(Debug, Error)]
pub enum YnabError {
    #[error("YNAB request failed: {0}")]
    Network(String),
    #[error("{message}")]
    Api { status: u16, message: String },
    #[error(
        "local YNAB rate limit reached ({max} requests/hour). Retry in {retry_after_secs}s. Narrow the query or wait before writing."
    )]
    RateLimited { max: u32, retry_after_secs: u64 },
}

impl YnabError {
    fn is_not_found(&self) -> bool {
        matches!(self, Self::Api { status: 404, .. })
    }
}

#[derive(Clone, Copy)]
pub enum CacheMode {
    Use,
    Bypass,
}

#[derive(Clone)]
pub struct YnabClient {
    http: reqwest::Client,
    base: reqwest::Url,
    api_key: Secret,
    default_plan: Option<String>,
    cache_ttl: Duration,
    cache: std::sync::Arc<Mutex<HashMap<String, (Instant, Value)>>>,
    max_requests_per_hour: u32,
    hits: std::sync::Arc<Mutex<VecDeque<Instant>>>,
}

impl YnabClient {
    pub fn from_config(config: &Config) -> Result<Self, String> {
        let mut base = reqwest::Url::parse(&config.ynab_api_base)
            .map_err(|error| format!("invalid YNAB API base: {error}"))?;
        let path = base.path().trim_end_matches('/').to_string();
        base.set_path(&format!("{path}/"));
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("ynab-mcp/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| format!("failed to build HTTP client: {error}"))?;
        Ok(Self {
            http,
            base,
            api_key: config.ynab_api_key.clone(),
            default_plan: config.ynab_plan_id.clone(),
            cache_ttl: config.cache_ttl,
            cache: std::sync::Arc::new(Mutex::new(HashMap::new())),
            max_requests_per_hour: config.max_requests_per_hour,
            hits: std::sync::Arc::new(Mutex::new(VecDeque::new())),
        })
    }

    pub fn default_plan(&self) -> Option<&str> {
        self.default_plan.as_deref()
    }

    pub async fn get(
        &self,
        path: &str,
        query: &[(&str, &str)],
        cache: CacheMode,
    ) -> Result<Value, YnabError> {
        self.send(Method::Get, path, query, None, cache).await
    }

    /// List endpoints use 404 for an empty collection. Treat that as no data.
    pub async fn get_optional(
        &self,
        path: &str,
        query: &[(&str, &str)],
        cache: CacheMode,
    ) -> Result<Option<Value>, YnabError> {
        match self.get(path, query, cache).await {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.is_not_found() => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub async fn post(&self, path: &str, body: Value) -> Result<Value, YnabError> {
        self.send(Method::Post, path, &[], Some(body), CacheMode::Bypass)
            .await
    }

    pub async fn put(&self, path: &str, body: Value) -> Result<Value, YnabError> {
        self.send(Method::Put, path, &[], Some(body), CacheMode::Bypass)
            .await
    }

    pub async fn patch(&self, path: &str, body: Value) -> Result<Value, YnabError> {
        self.send(Method::Patch, path, &[], Some(body), CacheMode::Bypass)
            .await
    }

    pub async fn delete(&self, path: &str) -> Result<Value, YnabError> {
        self.send(Method::Delete, path, &[], None, CacheMode::Bypass)
            .await
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<Value>,
        cache: CacheMode,
    ) -> Result<Value, YnabError> {
        let key = cache_key(path, query);
        if matches!(method, Method::Get) && matches!(cache, CacheMode::Use) {
            if let Some(hit) = self.cache_get(&key) {
                return Ok(hit);
            }
        }
        self.acquire_rate_slot()?;
        let url = self
            .base
            .join(path)
            .map_err(|error| YnabError::Network(format!("invalid YNAB path: {error}")))?;
        let started = Instant::now();
        let mut request = self.http.request(method.reqwest(), url);
        if !query.is_empty() {
            request = request.query(query);
        }
        if let Some(body) = &body {
            request = request.json(body);
        }
        request = request.bearer_auth(self.api_key.expose());
        let response = request
            .send()
            .await
            .map_err(|error| YnabError::Network(sanitize_client_error(&error.to_string())))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| YnabError::Network(sanitize_client_error(&error.to_string())))?;
        tracing::info!(
            method = method.as_str(),
            path,
            status = status.as_u16(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "ynab request"
        );
        if !status.is_success() {
            return Err(YnabError::Api {
                status: status.as_u16(),
                message: format_api_error(status, &text),
            });
        }
        let value = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).map_err(|error| {
                YnabError::Network(format!("YNAB returned invalid JSON: {error}"))
            })?
        };
        if matches!(method, Method::Get) && matches!(cache, CacheMode::Use) {
            self.cache_put(key, value.clone());
        } else if !matches!(method, Method::Get) {
            self.cache_clear();
        }
        Ok(value)
    }

    fn acquire_rate_slot(&self) -> Result<(), YnabError> {
        if self.max_requests_per_hour == 0 {
            return Ok(());
        }
        let mut hits = self.hits.lock().unwrap_or_else(|error| error.into_inner());
        let now = Instant::now();
        let window = Duration::from_secs(3600);
        while hits
            .front()
            .is_some_and(|stamp| now.duration_since(*stamp) >= window)
        {
            hits.pop_front();
        }
        if hits.len() >= self.max_requests_per_hour as usize {
            let oldest = hits.front().copied().unwrap_or(now);
            let retry_after_secs = window
                .saturating_sub(now.duration_since(oldest))
                .as_secs()
                .max(1);
            return Err(YnabError::RateLimited {
                max: self.max_requests_per_hour,
                retry_after_secs,
            });
        }
        hits.push_back(now);
        Ok(())
    }

    fn cache_get(&self, key: &str) -> Option<Value> {
        if self.cache_ttl.is_zero() {
            return None;
        }
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        let now = Instant::now();
        match cache.get(key) {
            Some((expires, value)) if *expires > now => Some(value.clone()),
            Some(_) => {
                cache.remove(key);
                None
            }
            None => None,
        }
    }

    fn cache_put(&self, key: String, value: Value) {
        if self.cache_ttl.is_zero() {
            return;
        }
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        if cache.len() >= 128 {
            cache.clear();
        }
        cache.insert(key, (Instant::now() + self.cache_ttl, value));
    }

    fn cache_clear(&self) {
        self.cache
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
    }
}

#[derive(Clone, Copy)]
enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }

    fn reqwest(self) -> reqwest::Method {
        match self {
            Self::Get => reqwest::Method::GET,
            Self::Post => reqwest::Method::POST,
            Self::Put => reqwest::Method::PUT,
            Self::Patch => reqwest::Method::PATCH,
            Self::Delete => reqwest::Method::DELETE,
        }
    }
}

fn cache_key(path: &str, query: &[(&str, &str)]) -> String {
    let mut pairs = query.to_vec();
    pairs.sort_unstable();
    let encoded = pairs
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    format!("{path}?{encoded}")
}

fn format_api_error(status: StatusCode, body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        if let Some(error) = value.get("error") {
            let name = error.get("name").and_then(Value::as_str).unwrap_or("error");
            let detail = error
                .get("detail")
                .and_then(Value::as_str)
                .unwrap_or("the request was rejected");
            return format!("YNAB API {status} {name}: {detail}");
        }
    }
    let snippet: String = body.chars().take(300).collect();
    format!("YNAB API {status}: {snippet}")
}

fn sanitize_client_error(message: &str) -> String {
    message
        .split_whitespace()
        .filter(|part| !part.to_ascii_lowercase().contains("bearer"))
        .collect::<Vec<_>>()
        .join(" ")
}
