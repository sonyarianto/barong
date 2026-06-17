use crate::agent::conversation::Message;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub created: String,
    pub updated: String,
    pub messages: Vec<Message>,
}

pub struct SessionManager {
    dir: PathBuf,
}

impl SessionManager {
    pub fn new() -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".into());
        let dir = PathBuf::from(home).join(".kalicode").join("sessions");
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    pub fn session_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.json", id))
    }

    pub fn most_recent_session(&self) -> Option<Session> {
        let mut entries: Vec<_> = std::fs::read_dir(&self.dir).ok()?
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .collect();
        entries.sort_by_key(|e| e.path().metadata().ok().map(|m| m.modified().ok()));
        entries.reverse();
        let latest = entries.first()?;
        let content = std::fs::read_to_string(latest.path()).ok()?;
        serde_json::from_str(&content).ok()
    }

    pub fn save(&self, messages: &[Message]) -> String {
        let id = chrono_format();
        let now = human_time();
        let session = Session {
            id: id.clone(),
            created: now.clone(),
            updated: now,
            messages: messages.to_vec(),
        };
        let path = self.session_path(&id);
        if let Ok(json) = serde_json::to_string_pretty(&session) {
            let _ = std::fs::write(path, json);
        }
        id
    }

    pub fn update(&self, id: &str, messages: &[Message]) {
        let path = self.session_path(id);
        if !path.exists() {
            return;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(mut session) = serde_json::from_str::<Session>(&content) {
                session.messages = messages.to_vec();
                session.updated = human_time();
                if let Ok(json) = serde_json::to_string_pretty(&session) {
                    let _ = std::fs::write(&path, json);
                }
            }
        }
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
    format!("{}", dur.as_secs())
}
