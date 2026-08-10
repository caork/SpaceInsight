use std::f32;

/// Threshold used to mark tiny items (for rendering decisions only).
///
/// Important: layout must never expand rectangles to this size, otherwise
/// neighboring tiles can overlap.
pub const MIN_VISIBLE_SIZE: f32 = 16.0;

/// Row growth tolerance: allow a tiny score increase when adding an item,
/// which helps avoid overly fragmented strip-like rows.
const ROW_GROWTH_TOLERANCE: f32 = 1.03;

/// Direction scoring weights tuned to prefer near-square tiles while still
/// keeping future container quality reasonable.
const WORST_ITEM_WEIGHT: f32 = 1.6;
const AVG_ITEM_WEIGHT: f32 = 0.7;
const REMAINING_WEIGHT: f32 = 0.9;
const STRIP_PENALTY_START: f32 = 5.0;
const STRIP_PENALTY_WEIGHT: f32 = 2.2;

/// Rectangle structure for treemap layout
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn area(&self) -> f32 {
        self.width * self.height
    }

    pub fn aspect_ratio(&self) -> f32 {
        if self.width < 1.0 || self.height < 1.0 {
            f32::INFINITY
        } else {
            self.width.max(self.height) / self.width.min(self.height)
        }
    }
}

/// Item to be laid out in the treemap
#[derive(Debug, Clone)]
pub struct TreemapItem {
    pub size: u64,
    pub index: usize,
}

/// Result of the treemap layout calculation
#[derive(Debug, Clone)]
pub struct LayoutRect {
    pub rect: Rect,
    pub index: usize,
    /// True if item is below minimum visible size threshold
    pub is_tiny: bool,
}

/// Squarified Treemap with adaptive split direction.
///
/// Standard squarified treemap always splits along the shorter axis.
/// This version tries BOTH directions at each row placement and picks
/// the direction that produces the best-shaped remaining container,
/// preventing thin strips when a dominant item consumes most of the area.
pub struct SquarifiedTreemap;

impl SquarifiedTreemap {
    /// Calculate the squarified treemap layout
    pub fn layout(items: &[TreemapItem], container: Rect) -> Vec<LayoutRect> {
        if items.is_empty() {
            return vec![];
        }

        let total_size: u64 = items.iter().map(|item| item.size).sum();
        if total_size == 0 {
            return vec![];
        }

        // Normalize sizes to fit container area
        let scale = container.area() as f64 / total_size as f64;
        let mut normalized: Vec<(usize, f32)> = items
            .iter()
            .map(|item| (item.index, (item.size as f64 * scale) as f32))
            .collect();

        // Sort by size descending for better aspect ratios
        normalized.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        let mut result = Vec::with_capacity(items.len());
        Self::squarify(&normalized, &mut result, container);

        // Post-process: mark tiny items only (do NOT resize, to avoid overlaps)
        for lr in &mut result {
            lr.is_tiny = lr.rect.width < MIN_VISIBLE_SIZE || lr.rect.height < MIN_VISIBLE_SIZE;
        }

        result
    }

    fn squarify(items: &[(usize, f32)], result: &mut Vec<LayoutRect>, container: Rect) {
        if items.is_empty() {
            return;
        }

        let mut current_row = Vec::new();
        let mut remaining_container = container;
        let mut next_index = 0usize;

        while next_index < items.len() {
            let next = items[next_index];
            if current_row.is_empty() {
                current_row.push(next);
                next_index += 1;
                continue;
            }

            let current_score = Self::best_row_score(&current_row, remaining_container);
            let mut test_row = current_row.clone();
            test_row.push(next);
            let test_score = Self::best_row_score(&test_row, remaining_container);

            if test_score <= current_score * ROW_GROWTH_TOLERANCE {
                current_row.push(next);
                next_index += 1;
            } else {
                let row_total: f32 = current_row.iter().map(|(_, size)| size).sum();
                let horizontal = Self::pick_direction(&current_row, remaining_container, row_total);
                let new_container =
                    Self::compute_remaining(remaining_container, row_total, horizontal);
                Self::emit_row(&current_row, result, remaining_container, horizontal);
                current_row.clear();
                remaining_container = new_container;
            }
        }

        if !current_row.is_empty() {
            Self::emit_row(
                &current_row,
                result,
                remaining_container,
                remaining_container.width >= remaining_container.height,
            );
        }
    }

    /// Try both split directions, return true for horizontal, false for vertical.
    /// Picks the direction that gives the best combination of:
    ///   - aspect ratios of items in the current row
    ///   - aspect ratio of the remaining container (weighted 2x)
    fn pick_direction(row: &[(usize, f32)], container: Rect, total: f32) -> bool {
        if total <= 0.0 || container.width <= 0.0 || container.height <= 0.0 {
            return container.width >= container.height;
        }

        let score_h = Self::direction_score(row, container, total, true);
        let score_v = Self::direction_score(row, container, total, false);

        // Lower score is better; prefer horizontal on tie
        score_h <= score_v
    }

    /// Score the row by taking the better of horizontal/vertical placement.
    fn best_row_score(row: &[(usize, f32)], container: Rect) -> f32 {
        let total: f32 = row.iter().map(|(_, size)| size).sum();
        if total <= 0.0 || container.width <= 0.0 || container.height <= 0.0 {
            return f32::INFINITY;
        }

        let score_h = Self::direction_score(row, container, total, true);
        let score_v = Self::direction_score(row, container, total, false);
        score_h.min(score_v)
    }

    /// Score a direction: lower is better.
    ///
    /// This favors:
    /// - low worst/average item aspect ratio (more square-like tiles)
    /// - reasonable remaining container aspect ratio
    /// - explicit penalty for extreme strip-like tiles
    fn direction_score(row: &[(usize, f32)], container: Rect, total: f32, horizontal: bool) -> f32 {
        let length = if horizontal {
            container.width
        } else {
            container.height
        };
        let breadth = if length > 0.0 { total / length } else { 0.0 };

        // Item aspect stats for this direction
        let mut worst_item = 1.0f32;
        let mut aspect_sum = 0.0f32;
        let mut aspect_count = 0.0f32;

        for &(_, size) in row {
            let item_len = if total > 0.0 {
                size / total * length
            } else {
                0.0
            };
            if item_len > 0.001 && breadth > 0.001 {
                let a = item_len.max(breadth) / item_len.min(breadth);
                worst_item = worst_item.max(a);
                aspect_sum += a;
                aspect_count += 1.0;
            }
        }

        let avg_item = if aspect_count > 0.0 {
            aspect_sum / aspect_count
        } else {
            1.0
        };

        // Aspect ratio of the remaining container
        let remaining = Self::compute_remaining(container, total, horizontal);
        let rem_aspect = remaining.aspect_ratio();
        let rem_aspect = if rem_aspect.is_infinite() {
            1.0
        } else {
            rem_aspect
        };

        let strip_penalty = if worst_item > STRIP_PENALTY_START {
            (worst_item - STRIP_PENALTY_START) * STRIP_PENALTY_WEIGHT
        } else {
            0.0
        };

        WORST_ITEM_WEIGHT * worst_item
            + AVG_ITEM_WEIGHT * avg_item
            + REMAINING_WEIGHT * rem_aspect
            + strip_penalty
    }

    /// Place items into a row in the given direction.
    fn emit_row(
        row: &[(usize, f32)],
        result: &mut Vec<LayoutRect>,
        container: Rect,
        horizontal: bool,
    ) {
        let total: f32 = row.iter().map(|(_, size)| size).sum();

        let length = if horizontal {
            container.width
        } else {
            container.height
        };

        let max_breadth = if horizontal {
            container.height
        } else {
            container.width
        };

        let row_breadth = if total > 0.0 && length > 0.0 {
            (total / length).min(max_breadth)
        } else {
            0.0
        };

        let mut offset = 0.0f32;
        let mut cumulative_size = 0.0f64;

        for (row_index, &(index, size)) in row.iter().enumerate() {
            cumulative_size += size as f64;
            let item_end = if row_index + 1 == row.len() {
                length
            } else if total > 0.0 {
                ((cumulative_size / total as f64) * length as f64) as f32
            } else {
                0.0
            }
            .clamp(offset, length);
            let item_length = item_end - offset;

            let rect = if horizontal {
                Rect::new(container.x + offset, container.y, item_length, row_breadth)
            } else {
                Rect::new(container.x, container.y + offset, row_breadth, item_length)
            };

            result.push(LayoutRect {
                rect,
                index,
                is_tiny: false,
            });
            offset = item_end;
        }
    }

    /// Compute the remaining container after placing a row.
    fn compute_remaining(container: Rect, row_total: f32, horizontal: bool) -> Rect {
        let length = if horizontal {
            container.width
        } else {
            container.height
        };

        let max_breadth = if horizontal {
            container.height
        } else {
            container.width
        };

        let row_breadth = if row_total > 0.0 && length > 0.0 {
            (row_total / length).min(max_breadth)
        } else {
            0.0
        };

        if horizontal {
            Rect::new(
                container.x,
                container.y + row_breadth,
                container.width,
                (container.height - row_breadth).max(0.0),
            )
        } else {
            Rect::new(
                container.x + row_breadth,
                container.y,
                (container.width - row_breadth).max(0.0),
                container.height,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_treemap_basic() {
        let items = vec![
            TreemapItem {
                size: 100,
                index: 0,
            },
            TreemapItem {
                size: 200,
                index: 1,
            },
            TreemapItem {
                size: 300,
                index: 2,
            },
        ];

        let container = Rect::new(0.0, 0.0, 800.0, 600.0);
        let layout = SquarifiedTreemap::layout(&items, container);

        assert_eq!(layout.len(), 3);
        // Verify total area is approximately preserved
        let total_area: f32 = layout.iter().map(|r| r.rect.area()).sum();
        let ratio = total_area / container.area();
        assert!(
            ratio > 0.99 && ratio < 1.01,
            "Total area ratio {} should be close to 1.0",
            ratio
        );
    }

    #[test]
    fn test_dominant_item_no_thin_strip() {
        // Simulates the user's case: one huge item + a few small ones
        let items = vec![
            TreemapItem {
                size: 6476,
                index: 0,
            }, // "github" ~90%
            TreemapItem {
                size: 641,
                index: 1,
            }, // "DCIM" ~9%
            TreemapItem { size: 50, index: 2 },
            TreemapItem { size: 30, index: 3 },
        ];

        let container = Rect::new(0.0, 0.0, 1200.0, 780.0);
        let layout = SquarifiedTreemap::layout(&items, container);

        assert_eq!(layout.len(), 4);

        // All rects should have area > 0
        for lr in &layout {
            assert!(lr.rect.area() > 0.0);
        }

        // The second-largest item (DCIM) should not be excessively thin.
        // With 90% dominant item, the best achievable DCIM aspect ratio is ~5.8
        // (constrained by the remaining strip width). This is much better than
        // the old horizontal-only layout which gave ~17:1.
        let dcim = layout.iter().find(|lr| lr.index == 1).unwrap();
        let dcim_aspect =
            dcim.rect.width.max(dcim.rect.height) / dcim.rect.width.min(dcim.rect.height);
        assert!(
            dcim_aspect < 8.0,
            "DCIM aspect ratio {} is too elongated",
            dcim_aspect
        );

        // Total area should still be preserved
        let total_area: f32 = layout.iter().map(|r| r.rect.area()).sum();
        let ratio = total_area / container.area();
        assert!(
            ratio > 0.99 && ratio < 1.01,
            "Total area ratio {} should be close to 1.0",
            ratio
        );
    }

    #[test]
    fn test_many_small_items_do_not_overlap() {
        let mut items = vec![TreemapItem {
            size: 5_000,
            index: 0,
        }];
        for index in 1..50 {
            items.push(TreemapItem {
                size: 8 + index as u64,
                index,
            });
        }

        let container = Rect::new(0.0, 0.0, 1200.0, 780.0);
        let layout = SquarifiedTreemap::layout(&items, container);

        assert_eq!(layout.len(), items.len());

        for lr in &layout {
            assert!(lr.rect.x >= container.x - 0.001);
            assert!(lr.rect.y >= container.y - 0.001);
            assert!(lr.rect.x + lr.rect.width <= container.x + container.width + 0.001);
            assert!(lr.rect.y + lr.rect.height <= container.y + container.height + 0.001);
        }

        for i in 0..layout.len() {
            for j in (i + 1)..layout.len() {
                let a = layout[i].rect;
                let b = layout[j].rect;

                let overlap_w = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
                let overlap_h = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);

                // Touching edges is fine; positive overlap area is not.
                assert!(
                    !(overlap_w > 0.01 && overlap_h > 0.01),
                    "Rects {} and {} overlap",
                    i,
                    j
                );
            }
        }
    }

    #[test]
    fn test_row_breadth_is_clamped_to_remaining_container() {
        let container = Rect::new(10.0, 20.0, 4.0, 3.0);
        let row = vec![(0, 10.0), (1, 10.0)];
        let mut layout = Vec::new();

        SquarifiedTreemap::emit_row(&row, &mut layout, container, true);

        assert_eq!(layout.len(), 2);
        assert!(layout.iter().all(|item| item.rect.y == container.y));
        assert!(layout
            .iter()
            .all(|item| item.rect.height == container.height));
        assert!(layout
            .iter()
            .all(|item| item.rect.x + item.rect.width <= container.x + container.width));
    }

    #[test]
    fn test_large_flat_directory_uses_iterative_layout() {
        let items: Vec<TreemapItem> = (0..20_000)
            .map(|index| TreemapItem {
                size: (index % 97 + 1) as u64,
                index,
            })
            .collect();

        let container = Rect::new(0.0, 0.0, 1600.0, 900.0);
        let layout = SquarifiedTreemap::layout(&items, container);

        assert_eq!(layout.len(), items.len());
        assert!(layout.iter().all(|item| item.rect.area() >= 0.0));

        for item in &layout {
            let rect = item.rect;
            assert!(rect.x >= container.x - 0.001);
            assert!(rect.y >= container.y - 0.001);
            assert!(
                rect.x + rect.width <= container.x + container.width + 0.01,
                "rect {:?} exceeds right edge {}",
                rect,
                container.x + container.width
            );
            assert!(
                rect.y + rect.height <= container.y + container.height + 0.01,
                "rect {:?} exceeds bottom edge {}",
                rect,
                container.y + container.height
            );
        }

        // With every rectangle bounded by the container, excess total area is
        // a strong signal that rows overlap due to accumulated float error.
        let total_area: f64 = layout.iter().map(|item| item.rect.area() as f64).sum();
        assert!(
            total_area <= container.area() as f64 * 1.000_01,
            "layout area {total_area} exceeds container area {}",
            container.area()
        );

        let mut by_left: Vec<Rect> = layout.iter().map(|item| item.rect).collect();
        by_left.sort_by(|a, b| a.x.total_cmp(&b.x));
        let mut active: Vec<Rect> = Vec::new();
        for rect in by_left {
            active.retain(|candidate| candidate.x + candidate.width > rect.x + 0.01);
            for candidate in &active {
                let overlap_height = (candidate.y + candidate.height).min(rect.y + rect.height)
                    - candidate.y.max(rect.y);
                assert!(
                    overlap_height <= 0.01,
                    "rectangles {:?} and {:?} overlap",
                    candidate,
                    rect
                );
            }
            active.push(rect);
        }
    }
}
