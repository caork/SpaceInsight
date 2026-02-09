use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Tracks which folders are expanded and to what depth.
pub struct ExpansionState {
    expanded: HashMap<PathBuf, u8>, // path -> expansion depth
}

impl Default for ExpansionState {
    fn default() -> Self {
        Self {
            expanded: HashMap::new(),
        }
    }
}

impl ExpansionState {
    /// Expand a folder to depth 1 (single-click).
    pub fn expand(&mut self, path: &Path) {
        self.expanded.insert(path.to_path_buf(), 1);
    }

    /// Increment expansion depth by 1 (double-click).
    pub fn deepen(&mut self, path: &Path) {
        let current = self.expanded.get(path).copied().unwrap_or(0);
        let new_depth = current.saturating_add(1);
        self.expanded.insert(path.to_path_buf(), new_depth);
    }

    /// Remove this path and all descendants from expanded set.
    pub fn collapse_recursive(&mut self, path: &Path) {
        let path_buf = path.to_path_buf();
        self.expanded.remove(&path_buf);
        // Remove all descendants
        self.expanded.retain(|p, _| !p.starts_with(&path_buf) || *p == path_buf);
        // The above retain keeps non-descendants. But we already removed path_buf,
        // so just remove anything that starts_with path_buf.
        self.expanded.retain(|p, _| !p.starts_with(&path_buf));
    }

    /// Get current expansion depth (0 = collapsed).
    pub fn depth(&self, path: &Path) -> u8 {
        self.expanded.get(path).copied().unwrap_or(0)
    }

    /// Check if a path is expanded.
    pub fn is_expanded(&self, path: &Path) -> bool {
        self.depth(path) > 0
    }

    /// Reset all expansions.
    pub fn collapse_all(&mut self) {
        self.expanded.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_collapse() {
        let mut state = ExpansionState::default();
        let path = PathBuf::from("/test/dir");

        assert!(!state.is_expanded(&path));
        assert_eq!(state.depth(&path), 0);

        state.expand(&path);
        assert!(state.is_expanded(&path));
        assert_eq!(state.depth(&path), 1);

        state.collapse_recursive(&path);
        assert!(!state.is_expanded(&path));
    }

    #[test]
    fn test_deepen() {
        let mut state = ExpansionState::default();
        let path = PathBuf::from("/test/dir");

        state.deepen(&path);
        assert_eq!(state.depth(&path), 1);

        state.deepen(&path);
        assert_eq!(state.depth(&path), 2);

        // Should keep increasing (with saturating add at u8 max)
        for _ in 0..10 {
            state.deepen(&path);
        }
        assert_eq!(state.depth(&path), 12);
    }

    #[test]
    fn test_collapse_recursive_removes_descendants() {
        let mut state = ExpansionState::default();
        let parent = PathBuf::from("/test/dir");
        let child = PathBuf::from("/test/dir/sub");
        let grandchild = PathBuf::from("/test/dir/sub/deep");
        let sibling = PathBuf::from("/test/other");

        state.expand(&parent);
        state.expand(&child);
        state.expand(&grandchild);
        state.expand(&sibling);

        state.collapse_recursive(&parent);

        assert!(!state.is_expanded(&parent));
        assert!(!state.is_expanded(&child));
        assert!(!state.is_expanded(&grandchild));
        assert!(state.is_expanded(&sibling)); // unaffected
    }

    #[test]
    fn test_collapse_all() {
        let mut state = ExpansionState::default();
        state.expand(&PathBuf::from("/a"));
        state.expand(&PathBuf::from("/b"));
        state.expand(&PathBuf::from("/c"));

        state.collapse_all();
        assert!(!state.is_expanded(&PathBuf::from("/a")));
        assert!(!state.is_expanded(&PathBuf::from("/b")));
        assert!(!state.is_expanded(&PathBuf::from("/c")));
    }
}
