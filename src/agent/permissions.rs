use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    AllowOnce,
    AllowSession,
    Deny,
}

#[derive(Debug, Clone)]
pub struct PendingTool {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

#[derive(Debug, Default)]
struct State {
    pending: Option<PendingTool>,
    decisions: HashMap<String, Decision>,
    session_allowed: HashSet<String>,
    auto_approve: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PermissionGate {
    inner: Arc<Mutex<State>>,
}

impl PermissionGate {
    pub fn new(auto_approve: bool) -> Self {
        Self {
            inner: Arc::new(Mutex::new(State {
                auto_approve,
                ..Default::default()
            })),
        }
    }

    pub fn set_auto_approve(&self, v: bool) {
        if let Ok(mut s) = self.inner.lock() {
            s.auto_approve = v;
        }
    }

    pub fn auto_approve(&self) -> bool {
        self.inner.lock().map(|s| s.auto_approve).unwrap_or(false)
    }

    /// Tools that mutate state require explicit approval.
    /// Read-only tools (read, grep, glob) are always allowed.
    pub fn requires_approval(tool_name: &str) -> bool {
        matches!(tool_name, "write" | "edit" | "bash" | "delegate")
    }

    pub fn is_session_allowed(&self, tool_name: &str) -> bool {
        self.inner
            .lock()
            .map(|s| s.session_allowed.contains(tool_name))
            .unwrap_or(false)
    }

    pub fn allow_session(&self, tool_name: &str) {
        if let Ok(mut s) = self.inner.lock() {
            s.session_allowed.insert(tool_name.to_string());
        }
    }

    pub fn request(&self, id: String, name: String, args: serde_json::Value) {
        if let Ok(mut s) = self.inner.lock() {
            s.pending = Some(PendingTool { id, name, args });
        }
    }

    pub fn pending(&self) -> Option<PendingTool> {
        self.inner.lock().ok()?.pending.clone()
    }

    pub fn clear_pending(&self) {
        if let Ok(mut s) = self.inner.lock() {
            s.pending = None;
        }
    }

    pub fn resolve(&self, id: &str, decision: Decision) {
        if let Ok(mut s) = self.inner.lock() {
            if decision == Decision::AllowSession {
                let name_opt = s.pending.as_ref().filter(|p| p.id == id).map(|p| p.name.clone());
                if let Some(name) = name_opt {
                    s.session_allowed.insert(name);
                }
            }
            s.decisions.insert(id.to_string(), decision);
            s.pending = None;
        }
    }

    pub fn poll(&self, id: &str) -> Option<Decision> {
        self.inner.lock().ok()?.decisions.remove(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_no_approval() {
        assert!(!PermissionGate::requires_approval("read"));
        assert!(!PermissionGate::requires_approval("grep"));
        assert!(!PermissionGate::requires_approval("glob"));
        assert!(PermissionGate::requires_approval("write"));
        assert!(PermissionGate::requires_approval("edit"));
        assert!(PermissionGate::requires_approval("bash"));
    }

    #[test]
    fn session_allow_flow() {
        let g = PermissionGate::new(false);
        assert!(!g.is_session_allowed("bash"));
        g.request("1".into(), "bash".into(), serde_json::json!({}));
        g.resolve("1", Decision::AllowSession);
        assert!(g.is_session_allowed("bash"));
        assert_eq!(g.poll("1"), Some(Decision::AllowSession));
    }
}
