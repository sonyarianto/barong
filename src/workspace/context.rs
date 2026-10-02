use std::path::PathBuf;

pub struct WorkspaceContext {
    pub root: PathBuf,
    pub git_branch: String,
    pub is_git_repo: bool,
    pub git_status: Vec<String>,
    pub file_tree: String,
}

static IGNORE_DIRS: &[&str] = &[
    ".git", "node_modules", "target", "build", "dist", ".next",
    "__pycache__", ".venv", "vendor", ".expo",
];

/// Project instructions. Checks git root + cwd, capped to ~4k chars.
pub fn load_agents_md(root: &std::path::Path) -> Option<String> {
    let candidates = [
        root.join("AGENTS.md"),
        std::env::current_dir().unwrap_or_default().join("AGENTS.md"),
    ];
    for p in candidates {
        if let Ok(raw) = std::fs::read_to_string(&p) {
            let trimmed = raw.trim().to_string();
            if trimmed.is_empty() {
                continue;
            }
            // cap ~4000 chars to protect context window
            let capped = if trimmed.len() > 4000 {
                format!("{}…\n[truncated {} chars]", &trimmed[..4000], trimmed.len() - 4000)
            } else {
                trimmed
            };
            return Some(capped);
        }
    }
    None
}

impl WorkspaceContext {
    pub fn new() -> Self {
        let root = Self::find_git_root().unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let mut ctx = Self {
            git_branch: String::new(),
            is_git_repo: false,
            git_status: Vec::new(),
            file_tree: String::new(),
            root,
        };

        ctx.detect_git();
        ctx.build_file_tree();
        ctx
    }

    fn find_git_root() -> Option<PathBuf> {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .ok()?;
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
        None
    }

    fn detect_git(&mut self) {
        let branch = std::process::Command::new("git")
            .args(["branch", "--show-current"])
            .output()
            .ok();
        if let Some(out) = branch {
            if out.status.success() {
                self.is_git_repo = true;
                self.git_branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
            }
        }

        if self.is_git_repo {
            let status = std::process::Command::new("git")
                .args(["status", "--short"])
                .output()
                .ok();
            if let Some(out) = status {
                let raw = String::from_utf8_lossy(&out.stdout);
                self.git_status = raw.lines().map(|l| l.to_string()).collect();
            }
        }
    }

    fn build_file_tree(&mut self) {
        let mut lines = Vec::new();
        self.walk_dir(&self.root, 0, 3, &mut lines);
        self.file_tree = lines.join("\n");
    }

    fn walk_dir(&self, dir: &std::path::Path, depth: usize, max_depth: usize, lines: &mut Vec<String>) {
        if depth > max_depth {
            return;
        }

        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        let mut dirs = Vec::new();
        let mut files = Vec::new();

        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            if name.starts_with('.') || IGNORE_DIRS.contains(&name.as_str()) {
                continue;
            }

            if path.is_dir() {
                dirs.push(name);
            } else {
                files.push(name);
            }
        }

        dirs.sort();
        files.sort();

        let indent = "  ".repeat(depth);
        for d in &dirs {
            lines.push(format!("{}{}/", indent, d));
            self.walk_dir(&dir.join(d), depth + 1, max_depth, lines);
        }
        for f in &files {
            lines.push(format!("{}{}", indent, f));
        }
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        parts.push(format!("Workspace root: {}", self.root.display()));

        if self.is_git_repo {
            parts.push(format!("Git branch: {}", self.git_branch));
            if !self.git_status.is_empty() {
                parts.push("Git status:".into());
                for line in &self.git_status {
                    parts.push(format!("  {}", line));
                }
            }
        }

        if let Some(agents) = load_agents_md(&self.root) {
            parts.push("\n## Project instructions (AGENTS.md)".into());
            parts.push(agents);
        }

        parts.push("\nFile tree:".into());
        if self.file_tree.is_empty() {
            parts.push("  (empty or ignored)".into());
        } else {
            parts.push(self.file_tree.clone());
        }

        parts.join("\n")
    }
}
