use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Live model discovery for OpenAI-compatible providers.
///
/// Instead of trusting the hardcoded catalog in `config.rs` (which rots —
/// models go EOL), ask the endpoint itself: `GET {base_url}/models` with the
/// stored key returns the actually-available ids. Anthropic has no such
/// endpoint, so it stays catalog-only.
pub const CACHE_TTL_SECS: u64 = 24 * 3600;
pub const MAX_DISCOVERED_PER_PROVIDER: usize = 100;

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    #[serde(default)]
    providers: HashMap<String, CachedProvider>,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
struct CachedProvider {
    #[serde(default)]
    fetched_at: u64,
    #[serde(default)]
    models: Vec<String>,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn cache_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".barong").join("models-cache.json")
}

/// Discovered models shared between the UI thread and background fetchers.
#[derive(Debug, Clone, Default)]
pub struct Discovered {
    inner: Arc<RwLock<HashMap<String, Vec<String>>>>,
}

impl Discovered {
    pub fn load() -> Self {
        let map = std::fs::read_to_string(cache_path())
            .ok()
            .and_then(|c| serde_json::from_str::<CacheFile>(&c).ok())
            .map(|f| {
                f.providers
                    .into_iter()
                    .map(|(k, v)| (k, v.models))
                    .collect()
            })
            .unwrap_or_default();
        Self { inner: Arc::new(RwLock::new(map)) }
    }

    pub fn get(&self, provider_id: &str) -> Vec<String> {
        self.inner
            .read()
            .ok()
            .and_then(|m| m.get(provider_id).cloned())
            .unwrap_or_default()
    }

    pub(crate) fn put(&self, provider_id: &str, models: Vec<String>) {
        if let Ok(mut m) = self.inner.write() {
            m.insert(provider_id.to_string(), models);
        }
        self.persist();
    }

    fn persist(&self) {
        let now = now_secs();
        let providers = self
            .inner
            .read()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| {
                        // Preserve the freshest fetched_at we know per provider.
                        let fetched_at = std::fs::read_to_string(cache_path())
                            .ok()
                            .and_then(|c| serde_json::from_str::<CacheFile>(&c).ok())
                            .and_then(|f| f.providers.get(k).map(|p| p.fetched_at))
                            .unwrap_or(now);
                        (
                            k.clone(),
                            CachedProvider { fetched_at, models: v.clone() },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let file = CacheFile { providers };
        if let Ok(json) = serde_json::to_string_pretty(&file) {
            if let Some(parent) = cache_path().parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(cache_path(), json);
        }
    }

    pub fn cache_age(&self, provider_id: &str) -> Option<u64> {
        std::fs::read_to_string(cache_path())
            .ok()
            .and_then(|c| serde_json::from_str::<CacheFile>(&c).ok())
            .and_then(|f| f.providers.get(provider_id).map(|p| now_secs().saturating_sub(p.fetched_at)))
    }
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    #[serde(default)]
    id: String,
}

pub fn parse_models_list(body: &str) -> Vec<String> {
    serde_json::from_str::<ModelsResponse>(body)
        .map(|r| {
            r.data
                .into_iter()
                .map(|e| e.id.trim().to_string())
                .filter(|id| !id.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

async fn fetch_async(base_url: &str, api_key: &str) -> anyhow::Result<Vec<String>> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(8)).build()?;
    let mut req = client.get(&url);
    if !api_key.trim().is_empty() {
        req = req.header("Authorization", format!("Bearer {}", api_key.trim()));
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("{} -> {}", url, resp.status());
    }
    let body = resp.text().await?;
    Ok(parse_models_list(&body))
}

/// Refresh one provider in the background (never blocks the TUI).
/// Anthropic and keyless non-public endpoints are skipped by the caller.
pub fn refresh_in_background(discovered: Discovered, provider_id: String, base_url: String, api_key: String) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(_) => return,
        };
        let models = rt.block_on(fetch_async(&base_url, &api_key)).unwrap_or_default();
        if models.is_empty() {
            return;
        }
        let mut models = models;
        models.truncate(MAX_DISCOVERED_PER_PROVIDER);
        // Stamp fresh fetch time by rewriting the cache entry.
        let path = cache_path();
        let mut file = std::fs::read_to_string(&path)
            .ok()
            .and_then(|c| serde_json::from_str::<CacheFile>(&c).ok())
            .unwrap_or_default();
        file.providers.insert(
            provider_id.clone(),
            CachedProvider { fetched_at: now_secs(), models: models.clone() },
        );
        if let Ok(json) = serde_json::to_string_pretty(&file) {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&path, json);
        }
        discovered.put(&provider_id, models);
        tracing::debug!("discovered models for '{}'", provider_id);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_shape() {
        let body = r#"{"object":"list","data":[{"id":"gpt-4o","object":"model"},{"id":"gpt-4o-mini"}]}"#;
        assert_eq!(parse_models_list(body), vec!["gpt-4o".to_string(), "gpt-4o-mini".to_string()]);
    }

    #[test]
    fn parses_openrouter_shape_and_rejects_garbage() {
        let body = r#"{"data":[{"id":"apodex/apodex-1.1-mini:free","name":"x"}]}"#;
        assert_eq!(parse_models_list(body), vec!["apodex/apodex-1.1-mini:free".to_string()]);
        assert!(parse_models_list("not json").is_empty());
        assert!(parse_models_list(r#"{"data":[]}"#).is_empty());
    }
}
