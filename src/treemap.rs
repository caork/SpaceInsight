use std::f32;

/// Threshold used to mark tiny items (for rendering decisions only).
///
/// Important: layout must never expand rectangles to this size, otherwise
/// neighboring tiles can overlap.
pub const MIN_VISIBLE_SIZE: f32 = 16.0;

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
        Self { x, y, width, height }
    }

    pub fn area(&self) -> f32 {
        self.width * self.height
    }

    pub fn short_side(&self) -> f32 {
        self.width.min(self.height)
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
        let mut remaining = items.to_vec();

        Self::squarify_recursive(&mut remaining, &mut current_row, result, container);
    }

    fn squarify_recursive(
        remaining: &mut Vec<(usize, f32)>,
        current_row: &mut Vec<(usize, f32)>,
        result: &mut Vec<LayoutRect>,
        container: Rect,
    ) {
        if remaining.is_empty() {
            if !current_row.is_empty() {
                // Last row: no remaining items, direction doesn't affect future layout
                Self::emit_row(current_row, result, container, container.width >= container.height);
                current_row.clear();
            }
            return;
        }

        let next = remaining[0];

        if current_row.is_empty() {
            current_row.push(next);
            remaining.remove(0);
            Self::squarify_recursive(remaining, current_row, result, container);
        } else {
            let current_worst = Self::worst_aspect_ratio(current_row, container);
            let mut test_row = current_row.clone();
            test_row.push(next);
            let test_worst = Self::worst_aspect_ratio(&test_row, container);

            if test_worst <= current_worst {
                current_row.push(next);
                remaining.remove(0);
                Self::squarify_recursive(remaining, current_row, result, container);
            } else {
                // Finalize this row: pick the best split direction
                let row_total: f32 = current_row.iter().map(|(_, size)| size).sum();
                let horizontal = Self::pick_direction(current_row, container, row_total);
                let new_container = Self::compute_remaining(container, row_total, horizontal);
                Self::emit_row(current_row, result, container, horizontal);
                current_row.clear();
                Self::squarify_recursive(remaining, current_row, result, new_container);
            }
        }
    }

    /// Try both split directions, return true for horizontal, false for vertical.
    /// Picks the direction that gives the best combination of:
    ///   - aspect ratios of items in the current row
    ///   - aspect ratio of the remaining container (weighted 2x)
    fn pick_direction(
        row: &[(usize, f32)],
        container: Rect,
        total: f32,
    ) -> bool {
        if total <= 0.0 || container.width <= 0.0 || container.height <= 0.0 {
            return container.width >= container.height;
        }

        let score_h = Self::direction_score(row, container, total, true);
        let score_v = Self::direction_score(row, container, total, false);

        // Lower score is better; prefer horizontal on tie
        score_h <= score_v
    }

    /// Score a direction: lower is better.
    /// Combines worst item aspect ratio + remaining container aspect ratio (weighted 2x).
    fn direction_score(
        row: &[(usize, f32)],
        container: Rect,
        total: f32,
        horizontal: bool,
    ) -> f32 {
        let length = if horizontal { container.width } else { container.height };
        let breadth = if length > 0.0 { total / length } else { 0.0 };

        // Worst aspect ratio of items in this row for this direction
        let mut worst_item = 1.0f32;
        for &(_, size) in row {
            let item_len = if total > 0.0 { size / total * length } else { 0.0 };
            if item_len > 0.001 && breadth > 0.001 {
                let a = item_len.max(breadth) / item_len.min(breadth);
                worst_item = worst_item.max(a);
            }
        }

        // Aspect ratio of the remaining container
        let remaining = Self::compute_remaining(container, total, horizontal);
        let rem_aspect = remaining.aspect_ratio();
        let rem_aspect = if rem_aspect.is_infinite() { 1.0 } else { rem_aspect };

        // Combined score: weight remaining container more heavily because
        // a bad remaining shape penalizes ALL subsequent items
        worst_item + 2.0 * rem_aspect
    }

    fn worst_aspect_ratio(row: &[(usize, f32)], container: Rect) -> f32 {
        if row.is_empty() {
            return f32::INFINITY;
        }

        let total: f32 = row.iter().map(|(_, size)| size).sum();
        let w = container.short_side();
        let max_size = row.iter().map(|(_, size)| size).fold(0.0f32, |a, &b| a.max(b));
        let min_size = row
            .iter()
            .map(|(_, size)| size)
            .fold(f32::INFINITY, |a, &b| a.min(b));

        let aspect1 = (w * w * max_size) / (total * total);
        let aspect2 = (total * total) / (w * w * min_size);

        aspect1.max(aspect2)
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

        let row_breadth = if total > 0.0 && length > 0.0 {
            total / length
        } else {
            0.0
        };

        let mut offset = 0.0f32;

        for (row_index, &(index, size)) in row.iter().enumerate() {
            let item_length = if row_index + 1 == row.len() {
                // Ensure the final item closes the row exactly; avoids float drift
                // causing tiny gaps or overlaps between neighbors.
                (length - offset).max(0.0)
            } else if total > 0.0 {
                size / total * length
            } else {
                0.0
            };

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
            offset += item_length;
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
            TreemapItem { size: 100, index: 0 },
            TreemapItem { size: 200, index: 1 },
            TreemapItem { size: 300, index: 2 },
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
            TreemapItem { size: 6476, index: 0 }, // "github" ~90%
            TreemapItem { size: 641, index: 1 },   // "DCIM" ~9%
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
        let dcim_aspect = dcim.rect.width.max(dcim.rect.height)
            / dcim.rect.width.min(dcim.rect.height);
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
        let mut items = vec![TreemapItem { size: 5_000, index: 0 }];
        for index in 1..50 {
            items.push(TreemapItem { size: 8 + index as u64, index });
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
}
