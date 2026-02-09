use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use crate::expand_state::ExpansionState;
use crate::tree::FileTree;
use crate::treemap::{Rect, SquarifiedTreemap, TreemapItem};

/// Visible border thickness (drawn).
pub const BORDER_VISUAL_WIDTH: f32 = 1.5;

/// Height of the header bar on expanded folders (label + collapse hit zone).
pub const HEADER_HEIGHT: f32 = 16.0;

/// Minimal inset on left/right/bottom of expanded folders.
pub const SIDE_INSET: f32 = 2.0;

/// Maximum expansion depth.
pub const MAX_EXPAND_DEPTH: u8 = 4;

// --- Aggregation parameters ---

/// Absolute minimum area (px²) for an item to render individually (~20x20).
const MIN_USEFUL_AREA: f32 = 400.0;

/// Minimum fraction of container area. Items below this are too small to see.
const MIN_AREA_PCT: f32 = 0.005; // 0.5% of container

/// Soft cap on individual items per level before aggregation kicks in.
const PREFERRED_MAX_ITEMS: usize = 12;

/// Maximum fraction of total size the grey block may consume.
/// If the aggregate would exceed this, items are rescued back out.
const MAX_AGGREGATE_FRACTION: f32 = 0.08; // 8%

/// A node in the render tree, produced by build_render_tree.
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
    /// Hash of path for stable egui IDs.
    pub stable_id: u64,
    /// True if this node represents aggregated small items.
    pub is_aggregate: bool,
    /// Number of items aggregated (only meaningful if is_aggregate).
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

/// Partition children into (kept individually, aggregated into grey block).
///
/// Three-phase algorithm:
///   Phase 1 — Area filter: items below the visibility threshold go to aggregate.
///   Phase 2 — Count cap: if too many items remain, move the smallest to aggregate.
///   Phase 3 — Budget rescue: if the aggregate is too large (>8% of total),
///             pull the largest aggregated items back out so the grey block stays small.
///
/// This ensures: small noise is swept up, the grey block never dominates,
/// and the visible layout has a manageable number of well-sized items.
fn partition_children(children: &[ChildInfo], total_size: u64, container_area: f32) -> (Vec<usize>, Vec<usize>) {
    let n = children.len();
    if n == 0 {
        return (vec![], vec![]);
    }
    // children are already sorted by size descending

    let min_area = MIN_USEFUL_AREA.max(container_area * MIN_AREA_PCT);

    // Phase 1: area filter
    let mut kept: Vec<usize> = Vec::new();
    let mut aggregated: Vec<usize> = Vec::new();

    for i in 0..n {
        let estimated_area = if total_size > 0 {
            children[i].size as f32 / total_size as f32 * container_area
        } else {
            0.0
        };
        if estimated_area >= min_area {
            kept.push(i);
        } else {
            aggregated.push(i);
        }
    }

    // Phase 2: count cap — if too many kept, move smallest to aggregate
    while kept.len() > PREFERRED_MAX_ITEMS {
        let removed = kept.pop().unwrap(); // smallest of kept (sorted desc)
        aggregated.insert(0, removed); // insert at front (it's the largest of aggregated)
    }

    // Phase 3: budget rescue — if aggregate too large, pull items back
    let budget = total_size as f64 * MAX_AGGREGATE_FRACTION as f64;
    let mut agg_total: u64 = aggregated.iter().map(|&i| children[i].size).sum();

    while agg_total as f64 > budget && !aggregated.is_empty() {
        // aggregated is ordered: largest first (from phase 2 inserts), then smallest
        // Pull the largest item (first element) back to kept
        let rescued = aggregated.remove(0);
        agg_total -= children[rescued].size;
        kept.push(rescued);
    }

    // Ensure at least 1 individual item
    if kept.is_empty() && !aggregated.is_empty() {
        let rescued = aggregated.remove(0);
        kept.push(rescued);
    }

    // Re-sort kept by size descending for consistent layout
    kept.sort_by(|&a, &b| children[b].size.cmp(&children[a].size));

    (kept, aggregated)
}

/// Build the hierarchical render tree from a FileTree.
pub fn build_render_tree(
    tree: &FileTree,
    root_id: indextree::NodeId,
    container: Rect,
    expansion: &ExpansionState,
    max_depth: u8,
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

    let total_size: u64 = children.iter().map(|c| c.size).sum();
    let container_area = container.area();

    let (kept_indices, agg_indices) = partition_children(&children, total_size, container_area);

    let aggregate_size: u64 = agg_indices.iter().map(|&i| children[i].size).sum();
    let aggregate_count = agg_indices.len();
    let has_aggregate = aggregate_count > 0 && aggregate_size > 0;

    let aggregate_path = container_root_path(root_id, arena);
    let aggregate_item_index = kept_indices.len(); // treemap index for aggregate

    // Build treemap items
    let mut items: Vec<TreemapItem> = kept_indices
        .iter()
        .enumerate()
        .map(|(treemap_idx, &child_idx)| TreemapItem {
            size: children[child_idx].size,
            index: treemap_idx,
        })
        .collect();

    if has_aggregate {
        items.push(TreemapItem {
            size: aggregate_size,
            index: aggregate_item_index,
        });
    }

    let layout = SquarifiedTreemap::layout(&items, container);

    let mut render_nodes = Vec::with_capacity(layout.len());

    for lr in &layout {
        let treemap_idx = lr.index;

        // Aggregate block
        if has_aggregate && treemap_idx == aggregate_item_index {
            let label = if aggregate_count == 1 {
                "1 small item".to_string()
            } else {
                format!("{} small items", aggregate_count)
            };
            render_nodes.push(RenderNode {
                path: aggregate_path.clone(),
                name: label,
                size: aggregate_size,
                is_dir: false,
                outer_rect: lr.rect,
                content_rect: None,
                children: Vec::new(),
                stable_id: path_hash(&aggregate_path) ^ 0xA66E,
                is_aggregate: true,
                aggregate_count,
            });
            continue;
        }

        if treemap_idx >= kept_indices.len() {
            continue;
        }
        let child_idx = kept_indices[treemap_idx];
        let child = &children[child_idx];

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
                build_render_tree(tree, child.node_id, cr, expansion, max_depth - 1)
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

/// Get a synthetic path for the aggregate node under this root.
fn container_root_path(
    root_id: indextree::NodeId,
    arena: &indextree::Arena<crate::tree::TreeNode>,
) -> PathBuf {
    arena
        .get(root_id)
        .map(|n| {
            let mut p = n.get().path.clone();
            p.push("__aggregate__");
            p
        })
        .unwrap_or_else(|| PathBuf::from("__aggregate__"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: simulate partition with given sizes.
    /// Uses a real Arena to get valid NodeIds.
    fn make_children(sizes: &[u64]) -> (indextree::Arena<crate::tree::TreeNode>, Vec<ChildInfo>) {
        let mut arena = indextree::Arena::new();
        let mut v: Vec<ChildInfo> = sizes
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                let node_id = arena.new_node(crate::tree::TreeNode {
                    path: PathBuf::from(format!("item_{}", i)),
                    name: format!("item_{}", i),
                    size: s,
                    cumulative_size: s,
                    is_dir: false,
                });
                ChildInfo {
                    node_id,
                    path: PathBuf::from(format!("item_{}", i)),
                    name: format!("item_{}", i),
                    size: s,
                    is_dir: false,
                }
            })
            .collect();
        v.sort_by(|a, b| b.size.cmp(&a.size));
        (arena, v)
    }

    #[test]
    fn test_dominant_item_small_aggregate() {
        // 90% dominant + 9% medium + 20 tiny items = 1%
        let mut sizes = vec![9000, 900];
        for _ in 0..20 {
            sizes.push(5); // 20 × 5 = 100 = 1%
        }
        let (_arena, children) = make_children(&sizes);
        let total: u64 = children.iter().map(|c| c.size).sum();
        let container_area = 936_000.0;

        let (kept, agg) = partition_children(&children, total, container_area);
        let agg_size: u64 = agg.iter().map(|&i| children[i].size).sum();
        let agg_frac = agg_size as f32 / total as f32;

        // The two big items should be kept; tiny items aggregated
        assert!(kept.len() >= 2, "Should keep at least the 2 large items, got {}", kept.len());
        // Aggregate should be small (≤ 8%)
        assert!(agg_frac <= MAX_AGGREGATE_FRACTION + 0.01,
                "Aggregate fraction {:.1}% exceeds budget", agg_frac * 100.0);
    }

    #[test]
    fn test_many_equal_items_aggregate_stays_small() {
        // 50 items of equal size — aggregate should not dominate
        let sizes: Vec<u64> = vec![100; 50];
        let (_arena, children) = make_children(&sizes);
        let total: u64 = children.iter().map(|c| c.size).sum();
        let container_area = 936_000.0;

        let (kept, agg) = partition_children(&children, total, container_area);
        let agg_size: u64 = agg.iter().map(|&i| children[i].size).sum();
        let agg_frac = agg_size as f32 / total as f32;

        // With 50 equal items, budget rescue should prevent the aggregate from being huge
        assert!(agg_frac <= MAX_AGGREGATE_FRACTION + 0.01,
                "Aggregate fraction {:.1}% exceeds budget", agg_frac * 100.0);
        assert!(kept.len() >= 1, "Should keep at least 1 item");
    }

    #[test]
    fn test_few_items_no_aggregate() {
        // 3 large items — nothing should be aggregated
        let sizes = vec![5000, 3000, 2000];
        let (_arena, children) = make_children(&sizes);
        let total: u64 = children.iter().map(|c| c.size).sum();
        let container_area = 936_000.0;

        let (kept, agg) = partition_children(&children, total, container_area);

        assert_eq!(kept.len(), 3);
        assert_eq!(agg.len(), 0);
    }
}
