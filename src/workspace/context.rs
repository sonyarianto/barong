use std::path::PathBuf;

pub struct WorkspaceContext {
    pub root: PathBuf,
    pub git_branch: String,
    pub is_git_repo: bool,
}

impl WorkspaceContext {
    pub fn new() -> Self {
        let root = std::env::current_dir().unwrap_or_default();
        Self {
            root,
            git_branch: String::new(),
            is_git_repo: false,
        }
    }

    pub fn summary(&self) -> String {
        format!(
            "Workspace: {}\nGit branch: {}",
            self.root.display(),
            self.git_branch
        )
    }
}
