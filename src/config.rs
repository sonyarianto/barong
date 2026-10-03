use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProviderConfig {
    /// API flavor: "openai" (default, chat completions) or "anthropic".
    pub api: Option<String>,
    pub base_url: Option<String>,
    /// Discouraged: prefer `~/.barong/auth.json` (`/login`) or env.
    /// Kept as last-resort fallback so old configs keep working.
    pub api_key: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub max_tokens: Option<u32>,
    pub theme: Option<String>,
    pub auto_approve: Option<bool>,
    pub auto_compact: Option<bool>,
    pub compact_keep: Option<usize>,
    /// Opt-in extras beyond core (read/write/edit/bash).
    /// e.g. ["grep", "glob", "delegate", "all"]
    #[serde(default)]
    pub extra_tools: Vec<String>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    /// Named providers (openrouter, deepseek, nvidia, ollama, ...).
    /// Merged over built-in defaults below; e.g.
    /// `"openrouter": {"base_url": "...", "models": ["x/y"]}`.
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
}

#[derive(Debug, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedProvider {
    pub api: String,
    pub base_url: String,
    pub models: Vec<String>,
    pub known: bool,
}

impl Config {
    pub fn load() -> Self {
        let paths = [
            Path::new("barong.jsonc"),
            Path::new("barong.json"),
        ];

        for path in &paths {
            if path.exists() {
                let raw = match std::fs::read_to_string(path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let cleaned = strip_jsonc_comments(&raw);
                match serde_json::from_str::<Config>(&cleaned) {
                    Ok(cfg) => {
                        tracing::debug!("Loaded config from {:?}", path);
                        return cfg;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse config {:?}: {}", path, e);
                    }
                }
            }
        }

        Self::default()
    }

    pub fn resolve_provider(&self) -> String {
        self.provider
            .clone()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::var("BARONG_PROVIDER").ok().map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "openai".into())
    }

    /// Built-in provider defaults: (api flavor, base_url, models, env var, needs_key).
    /// Covers OpenAI/Anthropic plus popular OpenAI-compatible endpoints
    /// (OpenRouter, DeepSeek, NVIDIA, Ollama) so reseller/proxy tokens work.
    pub fn known_provider(id: &str) -> Option<(&'static str, &'static str, &'static [&'static str], &'static str, bool)> {
        match id.trim().to_lowercase().as_str() {
            "openai" => Some(("openai", "https://api.openai.com/v1", &["gpt-4o", "gpt-4o-mini"], "OPENAI_API_KEY", true)),
            "anthropic" => Some(("anthropic", "", &["claude-sonnet-4-20250514"], "ANTHROPIC_API_KEY", true)),
            "openrouter" => Some(("openai", "https://openrouter.ai/api/v1",
                &["openai/gpt-4o", "anthropic/claude-sonnet-4", "deepseek/deepseek-chat-v3-0324", "qwen/qwen3-coder", "google/gemini-2.5-pro", "meta-llama/llama-3.3-70b-instruct"],
                "OPENROUTER_API_KEY", true)),
            "deepseek" => Some(("openai", "https://api.deepseek.com/v1",
                &["deepseek-chat", "deepseek-reasoner"], "DEEPSEEK_API_KEY", true)),
            "nvidia" => Some(("openai", "https://integrate.api.nvidia.com/v1",
                // Verified live 2026-10-02 with dummy-key probe (403 = exists,
                // 404/410 = dead). Catalogs rot — re-probe on doubt; users can
                // always type any `nvidia/<id>` free-form via `/model`.
                &["openai/gpt-oss-20b", "nvidia/llama-3.1-nemotron-70b-instruct"],
                "NVIDIA_API_KEY", true)),
            "ollama" => Some(("openai", "http://localhost:11434/v1",
                &["qwen2.5-coder:7b", "llama3.1:8b"], "", false)),
            // 9Router: local smart gateway (npm i -g 9router) routing 60+
            // providers with subscription/cheap/free fallback. Dashboard key.
            // Live ids come from discovery (/v1/models); catalog below is a
            // starter (free tier first = sensible default).
            "9router" => Some(("openai", "http://localhost:20128/v1",
                &["kr/claude-sonnet-4.5", "cc/claude-opus-4-5"],
                "NINEROUTER_API_KEY", true)),
            // Token Harbor: one universal key (thk_…) for all vendors,
            // OpenAI- + Anthropic-compatible. `:free` ids never charge.
            // Catalog from their quickstart; discovery fills the rest on login.
            "tokenharbor" => Some(("openai", "https://tokenharbor.ai/v1",
                &["deepseek-v4.1-flash", "claude-opus-5", "openai/gpt-4o-mini"],
                "TOKENHARBOR_API_KEY", true)),
            _ => None,
        }
    }

    pub fn known_provider_ids() -> Vec<&'static str> {
        vec!["openai", "anthropic", "openrouter", "deepseek", "nvidia", "ollama", "9router", "tokenharbor"]
    }

    /// All provider ids: built-ins plus user-defined in config.
    pub fn all_provider_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = Self::known_provider_ids().iter().map(|s| s.to_string()).collect();
        for k in self.providers.keys() {
            let id = k.trim().to_lowercase();
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids.sort();
        ids
    }
    /// Merge built-in defaults with user `providers` overrides.
    pub fn resolve_provider_config(&self, id: &str) -> ResolvedProvider {
        let id = id.trim().to_lowercase();
        let known = Self::known_provider(&id);
        let mut api = known.map(|k| k.0.to_string()).unwrap_or_else(|| "openai".into());
        let mut base_url = known.map(|k| k.1.to_string()).unwrap_or_default();
        let mut models: Vec<String> = known.map(|k| k.2.iter().map(|s| s.to_string()).collect()).unwrap_or_default();
        if let Some(o) = self.providers.get(&id) {
            if let Some(a) = &o.api {
                if !a.trim().is_empty() {
                    api = a.trim().to_lowercase();
                }
            }
            if let Some(u) = &o.base_url {
                if !u.trim().is_empty() {
                    base_url = u.trim().to_string();
                }
            }
            if !o.models.is_empty() {
                models = o.models.clone();
            }
        }
        // Legacy top-level base_url applies to the default provider.
        if id == self.resolve_provider() {
            if let Some(u) = &self.base_url {
                if !u.trim().is_empty() {
                    base_url = u.trim().to_string();
                }
            }
        }
        ResolvedProvider { api, base_url, models, known: known.is_some() || self.providers.contains_key(&id) }
    }

    /// Default model for a provider: config `model` (if default provider),
    /// else first known model, else "gpt-4o".
    pub fn resolve_default_model(&self, provider_id: &str) -> String {
        let id = provider_id.trim().to_lowercase();
        if id == self.resolve_provider() {
            if let Some(m) = &self.model {
                if !m.trim().is_empty() {
                    return m.trim().to_string();
                }
            }
        }
        let cfg = self.resolve_provider_config(&id);
        cfg.models.into_iter().next().unwrap_or_else(|| "gpt-4o".into())
    }

    pub fn resolve_model(&self, provider: &str) -> String {
        self.model
            .clone()
            .or_else(|| match provider {
                "anthropic" => std::env::var("ANTHROPIC_MODEL").ok(),
                _ => std::env::var("OPENAI_MODEL").ok(),
            })
            .unwrap_or_else(|| match provider {
                "anthropic" => "claude-sonnet-4-20250514".into(),
                _ => "gpt-4o".into(),
            })
    }

    pub fn resolve_api_key(&self, provider: &str) -> String {
        self.api_key
            .clone()
            .or_else(|| match provider {
                "anthropic" => {
                    std::env::var("ANTHROPIC_API_KEY")
                        .or_else(|_| std::env::var("BARONG_API_KEY"))
                        .ok()
                }
                _ => {
                    std::env::var("OPENAI_API_KEY")
                        .or_else(|_| std::env::var("BARONG_API_KEY"))
                        .ok()
                }
            })
            .unwrap_or_default()
    }

    pub fn resolve_base_url(&self) -> String {
        self.base_url
            .clone()
            .or_else(|| std::env::var("OPENAI_BASE_URL").ok())
            .unwrap_or_else(|| "https://api.openai.com/v1".into())
    }

    pub fn resolve_max_tokens(&self) -> u32 {
        self.max_tokens.unwrap_or(4096)
    }

    pub fn resolve_auto_approve(&self) -> bool {
        if let Some(v) = self.auto_approve {
            return v;
        }
        if let Ok(raw) = std::env::var("BARONG_AUTO_APPROVE") {
            let t = raw.trim().to_lowercase();
            return matches!(t.as_str(), "1" | "true" | "yes" | "y" | "on");
        }
        // Safe default: ask for mutating tools.
        false
    }

    pub fn resolve_auto_compact(&self) -> bool {
        if let Some(v) = self.auto_compact {
            return v;
        }
        if let Ok(raw) = std::env::var("BARONG_AUTO_COMPACT") {
            let t = raw.trim().to_lowercase();
            return !matches!(t.as_str(), "0" | "false" | "no" | "n" | "off");
        }
        true
    }

    pub fn resolve_compact_keep(&self) -> usize {
        if let Some(v) = self.compact_keep {
            return v.clamp(5, 100);
        }
        if let Ok(raw) = std::env::var("BARONG_COMPACT_KEEP") {
            if let Ok(v) = raw.trim().parse::<usize>() {
                return v.clamp(5, 100);
            }
        }
        20
    }

    pub fn resolve_theme(&self) -> String {
        if let Some(t) = &self.theme {
            if !t.trim().is_empty() {
                return t.trim().to_lowercase();
            }
        }
        if let Ok(s) = std::env::var("BARONG_THEME") {
            let t = s.trim().to_lowercase();
            if !t.is_empty() {
                return t;
            }
        }
        "dark".into()
    }

    pub fn resolve_extra_tools(&self) -> Vec<String> {
        if self.extra_tools.is_empty() {
            // Env override: BARONG_EXTRA_TOOLS="grep,glob,delegate" or "all"
            if let Ok(raw) = std::env::var("BARONG_EXTRA_TOOLS")
            {
                return raw
                    .split([',', ' '])
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect();
            }
        }
        self.extra_tools.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_defaults() {
        let cfg = Config::default();
        assert_eq!(cfg.resolve_provider(), "openai");
        assert_eq!(cfg.resolve_base_url(), "https://api.openai.com/v1");
        assert_eq!(cfg.resolve_model("openai"), "gpt-4o");
        assert!(cfg.resolve_extra_tools().is_empty());
    }

    #[test]
    fn test_extra_tools_env() {
        // SAFETY: unique var name, restored below; no other test reads it mid-flight.
        unsafe { std::env::set_var("BARONG_EXTRA_TOOLS", "grep,glob"); }
        let cfg = Config::default();
        let extras = cfg.resolve_extra_tools();
        assert!(extras.contains(&"grep".to_string()));
        unsafe { std::env::remove_var("BARONG_EXTRA_TOOLS"); }
    }

    #[test]
    fn test_provider_registry() {
        let cfg = Config::default();
        // Built-in reseller endpoints.
        let o = cfg.resolve_provider_config("openrouter");
        assert_eq!(o.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(o.api, "openai");
        assert!(o.models.iter().any(|m| m.contains("deepseek")));
        let d = cfg.resolve_provider_config("deepseek");
        assert!(d.models.contains(&"deepseek-chat".to_string()));
        let n = cfg.resolve_provider_config("nvidia");
        assert_eq!(n.base_url, "https://integrate.api.nvidia.com/v1");
        // 9Router: local OpenAI-compatible gateway, free tier first.
        let r = cfg.resolve_provider_config("9router");
        assert_eq!(r.api, "openai");
        assert_eq!(r.base_url, "http://localhost:20128/v1");
        assert_eq!(cfg.resolve_default_model("9router"), "kr/claude-sonnet-4.5");
        assert!(cfg.all_provider_ids().contains(&"9router".to_string()));
        // Token Harbor: universal key gateway, coder default.
        let th = cfg.resolve_provider_config("tokenharbor");
        assert_eq!(th.api, "openai");
        assert_eq!(th.base_url, "https://tokenharbor.ai/v1");
        assert_eq!(cfg.resolve_default_model("tokenharbor"), "deepseek-v4.1-flash");
        assert!(cfg.all_provider_ids().contains(&"tokenharbor".to_string()));
        // Unknown provider: openai-flavored, empty base/models.
        let x = cfg.resolve_provider_config("acme");
        assert!(!x.known);
        assert_eq!(cfg.resolve_default_model("deepseek"), "deepseek-chat");
        // User override merges over built-in.
        let mut custom = Config::default();
        custom.providers.insert("deepseek".into(), ProviderConfig {
            api: None,
            base_url: Some("https://proxy.local/v1".into()),
            api_key: None,
            models: vec!["custom-r1".into()],
        });
        let c = custom.resolve_provider_config("deepseek");
        assert_eq!(c.base_url, "https://proxy.local/v1");
        assert_eq!(c.models, vec!["custom-r1".to_string()]);
        assert!(custom.all_provider_ids().contains(&"deepseek".to_string()));
    }
}

fn strip_jsonc_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut string_char = '"';

    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
                continue;
            }
            if c == string_char {
                in_string = false;
            }
            continue;
        }

        match c {
            '"' | '\'' => {
                in_string = true;
                string_char = c;
                out.push(c);
            }
            '/' => {
                match chars.peek() {
                    Some(&'/') => {
                        while let Some(n) = chars.next() {
                            if n == '\n' {
                                out.push('\n');
                                break;
                            }
                        }
                    }
                    Some(&'*') => {
                        chars.next();
                        while let Some(n) = chars.next() {
                            if n == '*' && chars.peek() == Some(&'/') {
                                chars.next();
                                break;
                            }
                        }
                    }
                    _ => out.push(c),
                }
            }
            _ => out.push(c),
        }
    }
    out
}
