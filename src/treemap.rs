use std::f32;

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
}

/// Squarified Treemap Algorithm (Bruls, Huizing, van Wijk)
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
                Self::layout_row(current_row, result, container);
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
                // Layout current row and start new row
                let row_total: f32 = current_row.iter().map(|(_, size)| size).sum();
                let new_container = Self::get_remaining_rect(&container, row_total);
                Self::layout_row(current_row, result, container);
                current_row.clear();
                Self::squarify_recursive(remaining, current_row, result, new_container);
            }
        }
    }

    fn worst_aspect_ratio(row: &[(usize, f32)], container: Rect) -> f32 {
        if row.is_empty() {
            return f32::INFINITY;
        }

        let total: f32 = row.iter().map(|(_, size)| size).sum();
        let w = container.short_side();
        let max_size = row.iter().map(|(_, size)| size).fold(0.0f32, |a, &b| a.max(b));
        let min_size = row.iter().map(|(_, size)| size).fold(f32::INFINITY, |a, &b| a.min(b));

        let aspect1 = (w * w * max_size) / (total * total);
        let aspect2 = (total * total) / (w * w * min_size);
        
        aspect1.max(aspect2)
    }

    fn layout_row(row: &[(usize, f32)], result: &mut Vec<LayoutRect>, container: Rect) {
        let total: f32 = row.iter().map(|(_, size)| size).sum();
        
        let horizontal = container.width >= container.height;
        let (length, breadth) = if horizontal {
            (container.width, container.height)
        } else {
            (container.height, container.width)
        };
        
        let row_breadth = if total > 0.0 {
            total / length
        } else {
            0.0
        };

        let mut offset = 0.0f32;

        for &(index, size) in row {
            let item_length = if total > 0.0 {
                size / total * length
            } else {
                0.0
            };

            let rect = if horizontal {
                Rect::new(
                    container.x + offset,
                    container.y,
                    item_length,
                    row_breadth,
                )
            } else {
                Rect::new(
                    container.x,
                    container.y + offset,
                    row_breadth,
                    item_length,
                )
            };

            result.push(LayoutRect { rect, index });
            offset += item_length;
        }
    }

    fn get_remaining_rect(container: &Rect, row_total: f32) -> Rect {
        let horizontal = container.width >= container.height;
        let (length, breadth) = if horizontal {
            (container.width, container.height)
        } else {
            (container.height, container.width)
        };
        
        let row_breadth = if row_total > 0.0 {
            row_total / length
        } else {
            0.0
        };

        if horizontal {
            Rect::new(
                container.x,
                container.y + row_breadth,
                container.width,
                container.height - row_breadth,
            )
        } else {
            Rect::new(
                container.x + row_breadth,
                container.y,
                container.width - row_breadth,
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
        // Verify total area is approximately preserved (allowing for floating point errors)
        let total_area: f32 = layout.iter().map(|r| r.rect.area()).sum();
        let ratio = total_area / container.area();
        assert!(ratio > 0.99 && ratio < 1.01, 
                "Total area ratio {} should be close to 1.0", ratio);
    }
}
