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
            Path::new("kalicode.jsonc"),
            Path::new("kalicode.json"),
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
            .or_else(|| std::env::var("KALICODE_PROVIDER").ok())
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
                        .or_else(|_| std::env::var("KALICODE_API_KEY"))
                        .ok()
                }
                _ => {
                    std::env::var("OPENAI_API_KEY")
                        .or_else(|_| std::env::var("KALICODE_API_KEY"))
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_config() {
        let cfg = Config::load();
        assert_eq!(cfg.resolve_provider(), "openai");
        assert_eq!(cfg.resolve_base_url(), "https://openrouter.ai/api/v1");
        assert_eq!(cfg.resolve_model("openai"), "openrouter/free");
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
