use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Private per-provider API keys: `~/.barong/auth.json` (0600).
/// Never write keys to project `barong.jsonc` (it can be committed).
#[derive(Debug, Default, Serialize, Deserialize)]
struct AuthFile {
    #[serde(default)]
    keys: HashMap<String, String>,
}

pub struct AuthStore {
    path: PathBuf,
    keys: HashMap<String, String>,
}

impl AuthStore {
    pub fn new() -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".into());
        let path = PathBuf::from(home).join(".barong").join("auth.json");
        Self::load_from(path)
    }

    pub fn load_from(path: PathBuf) -> Self {
        let keys = std::fs::read_to_string(&path)
            .ok()
            .and_then(|c| serde_json::from_str::<AuthFile>(&c).ok())
            .map(|f| f.keys)
            .unwrap_or_default();
        Self { path, keys }
    }

    fn persist(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = AuthFile { keys: self.keys.clone() };
        if let Ok(json) = serde_json::to_string_pretty(&file) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                if let Ok(f) = std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&self.path)
                {
                    use std::io::Write;
                    let mut f = f;
                    let _ = f.write_all(json.as_bytes());
                    return;
                }
            }
            let _ = std::fs::write(&self.path, json);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
            }
        }
    }

    pub fn get(&self, provider_id: &str) -> Option<String> {
        let k = self.keys.get(&provider_id.trim().to_lowercase())?;
        if k.trim().is_empty() {
            return None;
        }
        Some(k.clone())
    }

    pub fn set(&mut self, provider_id: &str, api_key: &str) {
        self.keys.insert(
            provider_id.trim().to_lowercase(),
            api_key.trim().to_string(),
        );
        self.persist();
    }

    pub fn remove(&mut self, provider_id: &str) -> bool {
        let gone = self.keys.remove(&provider_id.trim().to_lowercase()).is_some();
        if gone {
            self.persist();
        }
        gone
    }

    pub fn has(&self, provider_id: &str) -> bool {
        self.get(provider_id).is_some()
    }
}

/// Resolve an API key for `provider_id`.
/// Order: `auth.json` (`/login`) > `<PROVIDER>_API_KEY` env (+ legacy
/// `OPENAI_`/`ANTHROPIC_`/`BARONG_API_KEY`) > `providers[id].api_key` in
/// project config (discouraged) > legacy top-level `api_key`.
/// Returns (key, source) where source is for status display.
pub fn resolve_api_key(
    provider_id: &str,
    auth: &AuthStore,
    config: &crate::config::Config,
) -> (String, &'static str) {
    let id = provider_id.trim().to_lowercase();
    if let Some(k) = auth.get(&id) {
        return (k, "auth.json");
    }
    // Provider-specific env, e.g. OPENROUTER_API_KEY, DEEPSEEK_API_KEY.
    let env_name = format!("{}_API_KEY", id.to_uppercase().replace('-', "_"));
    if let Ok(v) = std::env::var(&env_name) {
        if !v.trim().is_empty() {
            return (v, "env");
        }
    }
    // Legacy env names.
    let legacy: &[&str] = match id.as_str() {
        "anthropic" => &["ANTHROPIC_API_KEY", "BARONG_API_KEY"],
        "openai" => &["OPENAI_API_KEY", "BARONG_API_KEY"],
        _ => &["BARONG_API_KEY"],
    };
    for name in legacy {
        if let Ok(v) = std::env::var(name) {
            if !v.trim().is_empty() {
                return (v, "env");
            }
        }
    }
    if let Some(p) = config.providers.get(&id) {
        if let Some(k) = &p.api_key {
            if !k.trim().is_empty() {
                tracing::warn!(
                    "API key for '{}' lives in project config; move it to `/login` (auth.json) or env {}",
                    id,
                    env_name
                );
                return (k.trim().to_string(), "config");
            }
        }
    }
    if id == config.resolve_provider() {
        if let Some(k) = &config.api_key {
            if !k.trim().is_empty() {
                return (k.trim().to_string(), "config");
            }
        }
    }
    // Local servers need no real key; the endpoint ignores it.
    if id == "ollama" {
        return ("ollama".into(), "dummy");
    }
    (String::new(), "missing")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_remove() {
        let dir = std::env::temp_dir().join(format!("barong-auth-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("auth.json");
        let mut a = AuthStore::load_from(path.clone());
        assert!(a.get("openrouter").is_none());
        a.set("OpenRouter", "  sk-or-test  ");
        assert_eq!(a.get("openrouter").as_deref(), Some("sk-or-test"));
        let b = AuthStore::load_from(path.clone());
        assert_eq!(b.get("openrouter").as_deref(), Some("sk-or-test"));
        assert!(b.has("openrouter"));
        let mut b = b;
        assert!(b.remove("openrouter"));
        assert!(!b.has("openrouter"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn precedence_auth_over_env() {
        use crate::config::Config;
        let dir = std::env::temp_dir().join(format!("barong-auth-prec-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        // Unique provider id so the env var can't collide with other tests.
        // SAFETY: unique var name, no other test touches it.
        unsafe { std::env::set_var("TESTPROVXYZ_API_KEY", "from-env"); }
        let cfg = Config::default();
        let a = AuthStore::load_from(dir.join("auth.json"));
        let (k, src) = resolve_api_key("testprovxyz", &a, &cfg);
        assert_eq!((k.as_str(), src), ("from-env", "env"));
        let mut a = a;
        a.set("testprovxyz", "from-auth");
        let (k, src) = resolve_api_key("testprovxyz", &a, &cfg);
        assert_eq!((k.as_str(), src), ("from-auth", "auth.json"));
        unsafe { std::env::remove_var("TESTPROVXYZ_API_KEY"); }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
