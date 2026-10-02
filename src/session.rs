use crate::agent::conversation::Message;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub created: String,
    pub updated: String,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    /// Active endpoint when saved — restored on relaunch so a restart
    /// continues exactly where you left off (no silent reset to defaults).
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub theme: Option<String>,
}

/// Endpoint snapshot stored alongside a session.
#[derive(Debug, Default)]
pub struct SessionMeta {
    pub provider: String,
    pub model: String,
    pub theme: String,
}

pub struct SessionManager {
    dir: PathBuf,
}

impl SessionManager {
    pub fn new() -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".into());
        let dir = PathBuf::from(&home).join(".barong").join("sessions");
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    pub fn session_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.json", id))
    }

    pub fn most_recent_session(&self) -> Option<Session> {
        let mut list = self.list_sessions();
        list.sort_by(|a, b| b.updated.cmp(&a.updated));
        list.into_iter().next()
    }

    /// Most recent session that actually has content. Empty shells created by
    /// bare slash commands must never shadow real history on relaunch.
    pub fn most_recent_nonempty(&self) -> Option<Session> {
        let mut list = self.list_sessions();
        list.sort_by(|a, b| b.updated.cmp(&a.updated));
        list.into_iter().find(|s| !s.messages.is_empty())
    }

    pub fn list_sessions(&self) -> Vec<Session> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return vec![];
        };
        let mut out = Vec::new();
        for e in entries.flatten() {
            if e.path().extension().is_some_and(|ext| ext == "json") {
                if let Ok(content) = std::fs::read_to_string(e.path()) {
                    if let Ok(s) = serde_json::from_str::<Session>(&content) {
                        out.push(s);
                    }
                }
            }
        }
        out
    }

    pub fn load(&self, id_or_prefix: &str) -> Option<Session> {
        // exact, then prefix match
        let path = self.session_path(id_or_prefix);
        if path.exists() {
            if let Ok(c) = std::fs::read_to_string(&path) {
                if let Ok(s) = serde_json::from_str::<Session>(&c) {
                    return Some(s);
                }
            }
        }
        for s in self.list_sessions() {
            if s.id.starts_with(id_or_prefix) {
                return Some(s);
            }
        }
        None
    }

    pub fn export_jsonl(&self, id: &str, dest: &std::path::Path) -> anyhow::Result<()> {
        let content = std::fs::read_to_string(self.session_path(id))?;
        let session: Session = serde_json::from_str(&content)?;
        let mut out = String::new();
        for m in &session.messages {
            out.push_str(&serde_json::to_string(m)?);
            out.push('\n');
        }
        std::fs::write(dest, out)?;
        Ok(())
    }

    pub fn save(&self, messages: &[Message], meta: &SessionMeta) -> String {
        let id = chrono_format();
        let now = human_time();
        let session = Session {
            id: id.clone(),
            created: now.clone(),
            updated: now,
            messages: messages.to_vec(),
            parent_id: None,
            branch: None,
            provider: Some(meta.provider.clone()),
            model: Some(meta.model.clone()),
            theme: Some(meta.theme.clone()),
        };
        let path = self.session_path(&id);
        if let Ok(json) = serde_json::to_string_pretty(&session) {
            let _ = std::fs::write(path, json);
        }
        id
    }

    pub fn update(&self, id: &str, messages: &[Message], meta: &SessionMeta) {
        let path = self.session_path(id);
        if !path.exists() {
            return;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(mut session) = serde_json::from_str::<Session>(&content) {
                session.messages = messages.to_vec();
                session.updated = human_time();
                session.provider = Some(meta.provider.clone());
                session.model = Some(meta.model.clone());
                session.theme = Some(meta.theme.clone());
                if let Ok(json) = serde_json::to_string_pretty(&session) {
                    let _ = std::fs::write(&path, json);
                }
            }
        }
    }

    /// Fork current messages into a new branch session.
    pub fn fork(&self, messages: &[Message], parent_id: Option<&str>, branch: &str, meta: &SessionMeta) -> String {
        let id = chrono_format();
        let now = human_time();
        let session = Session {
            id: id.clone(),
            created: now.clone(),
            updated: now,
            messages: messages.to_vec(),
            parent_id: parent_id.map(|s| s.to_string()),
            branch: Some(branch.to_string()),
            provider: Some(meta.provider.clone()),
            model: Some(meta.model.clone()),
            theme: Some(meta.theme.clone()),
        };
        let path = self.session_path(&id);
        if let Ok(json) = serde_json::to_string_pretty(&session) {
            let _ = std::fs::write(path, json);
        }
        id
    }
    pub fn title(s: &Session) -> String {
        let first = s.messages.iter().find(|m| m.role == "user").and_then(|m| m.content.clone()).unwrap_or_default();
        let one_line: String = first.split_whitespace().collect::<Vec<_>>().join(" ");
        if one_line.is_empty() {
            return "(empty)".to_string();
        }
        if one_line.chars().count() <= 60 {
            return one_line;
        }
        format!("{}…", one_line.chars().take(59).collect::<String>())
    }
}

fn human_time() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs();
    let millis = dur.subsec_millis();
    format!("{}.{:03}", secs, millis)
}

fn chrono_format() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    format!("{}.{:03}", dur.as_secs(), dur.subsec_millis())
}
