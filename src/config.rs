use serde::Deserialize;
use std::path::Path;

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
}

#[derive(Debug, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
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
            .or_else(|| std::env::var("BARONG_PROVIDER").ok())
            .unwrap_or_else(|| "openai".into())
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
        std::env::set_var("BARONG_EXTRA_TOOLS", "grep,glob");
        let cfg = Config::default();
        let extras = cfg.resolve_extra_tools();
        assert!(extras.contains(&"grep".to_string()));
        std::env::remove_var("BARONG_EXTRA_TOOLS");
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
