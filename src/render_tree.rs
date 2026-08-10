use std::collections::{hash_map::DefaultHasher, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use crate::expand_state::ExpansionState;
use crate::tree::FileTree;
use crate::treemap::{Rect, SquarifiedTreemap, TreemapItem};

/// Visible border thickness (drawn).
pub const BORDER_VISUAL_WIDTH: f32 = 0.8;

/// Height of the header bar on expanded folders (label + collapse hit zone).
pub const HEADER_HEIGHT: f32 = 16.0;

/// Minimal inset on left/right/bottom of expanded folders.
pub const SIDE_INSET: f32 = 1.0;

/// A node in the render tree, produced by build_render_tree.
#[derive(Clone)]
pub struct RenderNode {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub outer_rect: Rect,
    /// Inner area for expanded folders; None if collapsed/file.
    pub content_rect: Option<Rect>,
    /// Sub-nodes (only populated for expanded folders).
    pub children: Vec<RenderNode>,
    /// Hash of path for stable frontend IDs.
    pub stable_id: u64,
    /// Legacy frontend compatibility; new layouts always return false.
    pub is_aggregate: bool,
    /// Legacy frontend compatibility; new layouts always return zero.
    pub aggregate_count: usize,
}

fn path_hash(path: &PathBuf) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

/// Collected child data before layout.
struct ChildInfo {
    node_id: indextree::NodeId,
    path: PathBuf,
    name: String,
    size: u64,
    is_dir: bool,
}

/// Build the hierarchical render tree from a FileTree.
pub fn build_render_tree(
    tree: &FileTree,
    root_id: indextree::NodeId,
    container: Rect,
    expansion: &ExpansionState,
    max_depth: usize,
    _split_small_items_roots: &HashSet<PathBuf>,
) -> Vec<RenderNode> {
    let arena = tree.get_arena();

    let mut children: Vec<ChildInfo> = root_id
        .children(arena)
        .filter_map(|child_id| {
            arena.get(child_id).map(|node| {
                let data = node.get();
                ChildInfo {
                    node_id: child_id,
                    path: data.path.clone(),
                    name: data.name.clone(),
                    size: data.cumulative_size,
                    is_dir: data.is_dir,
                }
            })
        })
        .collect();

    if children.is_empty() {
        return Vec::new();
    }

    // Sort by size descending
    children.sort_by(|a, b| b.size.cmp(&a.size));

    let items: Vec<TreemapItem> = children
        .iter()
        .enumerate()
        .map(|(index, child)| TreemapItem {
            size: child.size,
            index,
        })
        .collect();

    let layout = SquarifiedTreemap::layout(&items, container);

    let mut render_nodes = Vec::with_capacity(layout.len());

    for lr in &layout {
        if lr.index >= children.len() {
            continue;
        }
        let child = &children[lr.index];

        let exp_depth = expansion.depth(&child.path);
        let is_expanded = child.is_dir && exp_depth > 0 && max_depth > 0;

        let outer_rect = lr.rect;

        let (content_rect, sub_children) = if is_expanded {
            let cr = Rect::new(
                outer_rect.x + SIDE_INSET,
                outer_rect.y + HEADER_HEIGHT,
                (outer_rect.width - 2.0 * SIDE_INSET).max(1.0),
                (outer_rect.height - HEADER_HEIGHT - SIDE_INSET).max(1.0),
            );

            let sub = if cr.width > 4.0 && cr.height > 4.0 {
                build_render_tree(
                    tree,
                    child.node_id,
                    cr,
                    expansion,
                    max_depth - 1,
                    _split_small_items_roots,
                )
            } else {
                Vec::new()
            };

            (Some(cr), sub)
        } else {
            (None, Vec::new())
        };

        render_nodes.push(RenderNode {
            path: child.path.clone(),
            name: child.name.clone(),
            size: child.size,
            is_dir: child.is_dir,
            outer_rect,
            content_rect,
            children: sub_children,
            stable_id: path_hash(&child.path),
            is_aggregate: false,
            aggregate_count: 0,
        });
    }

    render_nodes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_render_tree_keeps_every_child() {
        let mut tree = FileTree::new("/test");
        for index in 0..50 {
            tree.upsert_node(PathBuf::from(format!("/test/file_{index}")), 100, false);
        }
        tree.calculate_sizes();

        let nodes = build_render_tree(
            &tree,
            tree.get_root(),
            Rect::new(0.0, 0.0, 1000.0, 700.0),
            &ExpansionState::default(),
            1,
            &HashSet::new(),
        );

        assert_eq!(nodes.len(), 50);
        assert!(nodes.iter().all(|node| !node.is_aggregate));
        let area: f32 = nodes.iter().map(|node| node.outer_rect.area()).sum();
        assert!((area - 700_000.0).abs() < 10.0);
    }
}
