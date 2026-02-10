use indextree::{Arena, NodeId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Represents a node in the directory tree
#[derive(Debug, Clone)]
pub struct TreeNode {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    /// Cumulative size including all children
    pub cumulative_size: u64,
}

/// High-performance hierarchical tree structure using an arena allocator
pub struct FileTree {
    arena: Arena<TreeNode>,
    root: NodeId,
    path_to_node: HashMap<PathBuf, NodeId>,
}

impl FileTree {
    /// Create a new tree with a root node
    pub fn new<P: AsRef<Path>>(root_path: P) -> Self {
        let mut arena = Arena::new();
        let root_path_buf = root_path.as_ref().to_path_buf();
        let root_name = root_path_buf
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("/")
            .to_string();

        let root_node = TreeNode {
            path: root_path_buf.clone(),
            name: root_name,
            size: 0,
            is_dir: true,
            cumulative_size: 0,
        };

        let root = arena.new_node(root_node);
        let mut path_to_node = HashMap::new();
        path_to_node.insert(root_path_buf, root);

        Self {
            arena,
            root,
            path_to_node,
        }
    }

    /// Calculate cumulative sizes for all directories (bottom-up)
    pub fn calculate_sizes(&mut self) {
        self.calculate_sizes_recursive(self.root);
    }

    fn calculate_sizes_recursive(&mut self, node_id: NodeId) -> u64 {
        let mut total = 0u64;

        // Collect children first (to avoid borrow issues)
        let children: Vec<NodeId> = node_id
            .children(&self.arena)
            .collect();

        // Recursively calculate sizes for children
        for child in children {
            total += self.calculate_sizes_recursive(child);
        }

        // Add own size
        if let Some(node) = self.arena.get_mut(node_id) {
            let node_data = node.get_mut();
            if node_data.is_dir {
                node_data.cumulative_size = total;
            } else {
                total += node_data.size;
                node_data.cumulative_size = node_data.size;
            }
        }

        total
    }

    pub fn get_root(&self) -> NodeId {
        self.root
    }

    pub fn get_arena(&self) -> &Arena<TreeNode> {
        &self.arena
    }

    pub fn get_node(&self, path: &Path) -> Option<NodeId> {
        self.path_to_node.get(path).copied()
    }

    pub fn root_path(&self) -> Option<&Path> {
        self.arena.get(self.root).map(|n| n.get().path.as_path())
    }

    fn ensure_directory_node(&mut self, path: &Path) -> Option<NodeId> {
        if let Some(node_id) = self.path_to_node.get(path).copied() {
            return Some(node_id);
        }

        let root_path = self.root_path()?.to_path_buf();
        if !path.starts_with(&root_path) {
            return None;
        }

        if path == root_path {
            return Some(self.root);
        }

        let parent_path = path.parent()?;
        let parent_id = self.ensure_directory_node(parent_path)?;

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        let node_id = self.arena.new_node(TreeNode {
            path: path.to_path_buf(),
            name,
            size: 0,
            is_dir: true,
            cumulative_size: 0,
        });

        parent_id.append(node_id, &mut self.arena);
        self.path_to_node.insert(path.to_path_buf(), node_id);
        Some(node_id)
    }

    pub fn upsert_node(&mut self, path: PathBuf, size: u64, is_dir: bool) {
        if self
            .root_path()
            .map(|root| path == root)
            .unwrap_or(false)
        {
            return;
        }

        let Some(root_path) = self.root_path().map(|p| p.to_path_buf()) else {
            return;
        };
        if !path.starts_with(&root_path) {
            return;
        }

        if is_dir {
            if let Some(node_id) = self.ensure_directory_node(&path) {
                if let Some(node) = self.arena.get_mut(node_id) {
                    let data = node.get_mut();
                    data.is_dir = true;
                    data.size = 0;
                }
            }
            return;
        }

        let Some(parent_path) = path.parent() else {
            return;
        };
        let Some(parent_id) = self.ensure_directory_node(parent_path) else {
            return;
        };

        if let Some(node_id) = self.path_to_node.get(&path).copied() {
            if let Some(node) = self.arena.get_mut(node_id) {
                let data = node.get_mut();
                data.is_dir = false;
                data.size = size;
                data.cumulative_size = size;
            }
            return;
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        let node_id = self.arena.new_node(TreeNode {
            path: path.clone(),
            name,
            size,
            is_dir: false,
            cumulative_size: size,
        });

        parent_id.append(node_id, &mut self.arena);
        self.path_to_node.insert(path, node_id);
    }

    pub fn remove_path_recursive(&mut self, path: &Path) -> bool {
        let Some(root_path) = self.root_path().map(|p| p.to_path_buf()) else {
            return false;
        };
        if path == root_path {
            return false;
        }

        let Some(node_id) = self.path_to_node.get(path).copied() else {
            return false;
        };

        let mut stack = vec![node_id];
        let mut ids = Vec::new();
        while let Some(id) = stack.pop() {
            ids.push(id);
            for child in id.children(&self.arena) {
                stack.push(child);
            }
        }

        for id in ids {
            if let Some(node) = self.arena.get(id) {
                self.path_to_node.remove(&node.get().path);
            }
        }

        node_id.detach(&mut self.arena);
        true
    }

    /// Get total size of the tree
    pub fn total_size(&self) -> u64 {
        self.arena
            .get(self.root)
            .map(|n| n.get().cumulative_size)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tree_basic() {
        let mut tree = FileTree::new("/test");
        tree.upsert_node(PathBuf::from("/test/file1.txt"), 100, false);
        tree.upsert_node(PathBuf::from("/test/dir1"), 0, true);
        tree.upsert_node(PathBuf::from("/test/dir1/file2.txt"), 200, false);
        
        tree.calculate_sizes();
        
        assert_eq!(tree.total_size(), 300);
    }
}
