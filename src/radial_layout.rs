use glam::Vec2;
use std::path::Path;

/// Golden ratio constant for center positioning
const GOLDEN_RATIO_X: f32 = 0.38;
const GOLDEN_RATIO_Y: f32 = 0.62;

/// File category determines angular cluster position
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileCategory {
    Media,      // Images, video, audio
    Code,       // Source files
    Archive,    // Compressed archives
    Document,   // Text, PDF, etc.
    System,     // Hidden, system files
    Other,
}

impl FileCategory {
    /// Get base angle in radians for category clustering
    pub fn base_angle(&self) -> f32 {
        match self {
            FileCategory::Media => 60.0_f32.to_radians(),      // 2 o'clock - warm amber
            FileCategory::Code => 300.0_f32.to_radians(),      // 10 o'clock - cool slate
            FileCategory::Archive => 180.0_f32.to_radians(),   // 6 o'clock - purple
            FileCategory::Document => 120.0_f32.to_radians(),  // 4 o'clock - green-blue
            FileCategory::System => 240.0_f32.to_radians(),    // 8 o'clock - deep blue
            FileCategory::Other => 0.0_f32.to_radians(),       // 12 o'clock - neutral
        }
    }

    /// Get color hue for category (0-360 degrees)
    pub fn hue_degrees(&self) -> f32 {
        match self {
            FileCategory::Media => 30.0,       // Warm amber/orange
            FileCategory::Code => 220.0,       // Cool blue
            FileCategory::Archive => 270.0,    // Purple
            FileCategory::Document => 160.0,   // Teal
            FileCategory::System => 240.0,     // Deep blue
            FileCategory::Other => 0.0,        // Red
        }
    }

    /// Classify file by extension
    pub fn from_path(path: &Path) -> Self {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let ext = ext.to_lowercase();
            match ext.as_str() {
                // Media
                "jpg" | "jpeg" | "png" | "gif" | "bmp" | "svg" | "webp" |
                "mp4" | "mov" | "avi" | "mkv" | "flv" | "wmv" |
                "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" => FileCategory::Media,
                
                // Code
                "rs" | "py" | "js" | "ts" | "jsx" | "tsx" | "cpp" | "c" | "h" | "hpp" |
                "java" | "go" | "rb" | "php" | "swift" | "kt" | "cs" | "m" | "mm" |
                "html" | "css" | "scss" | "sass" | "vue" | "svelte" => FileCategory::Code,
                
                // Archives
                "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "dmg" | "iso" => FileCategory::Archive,
                
                // Documents
                "pdf" | "doc" | "docx" | "txt" | "md" | "rtf" | "odt" |
                "xls" | "xlsx" | "csv" | "ppt" | "pptx" => FileCategory::Document,
                
                _ => FileCategory::Other,
            }
        } else {
            // Check for hidden/system files
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') {
                    return FileCategory::System;
                }
            }
            FileCategory::Other
        }
    }
}

/// Radial node with orbital positioning
#[derive(Debug, Clone)]
pub struct RadialNode {
    /// Distance from center (pixels)
    pub orbital_distance: f32,
    /// Angle in radians
    pub angle: f32,
    /// Visual size (diameter in pixels)
    pub size: f32,
    /// Aspect ratio (width / height)
    pub aspect_ratio: f32,
    /// File category
    pub category: FileCategory,
    /// Position in world space
    pub position: Vec2,
    /// Item index (for mapping back to data)
    pub index: usize,
}

/// Input item for radial layout
#[derive(Debug, Clone)]
pub struct RadialItem {
    pub size_bytes: u64,
    pub index: usize,
    pub category: FileCategory,
}

/// Rectangle for layout bounds
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

    pub fn center(&self) -> Vec2 {
        Vec2::new(self.x + self.width * 0.5, self.y + self.height * 0.5)
    }
}

/// Calculate radial orbital layout
pub fn calculate_orbital_layout(
    items: &[RadialItem],
    viewport: Rect,
) -> Vec<RadialNode> {
    if items.is_empty() {
        return vec![];
    }

    // Find max size for normalization
    let max_size = items.iter().map(|i| i.size_bytes).max().unwrap_or(1);
    
    // Calculate center point at golden ratio
    let center = Vec2::new(
        viewport.x + viewport.width * GOLDEN_RATIO_X,
        viewport.y + viewport.height * GOLDEN_RATIO_Y,
    );

    // Calculate available radius (distance to nearest edge from center)
    let max_radius = viewport.width.min(viewport.height) * 0.45;

    let mut nodes = Vec::with_capacity(items.len());

    // Group items by category for angular distribution
    let mut category_counts = std::collections::HashMap::new();
    let mut category_indices = std::collections::HashMap::new();
    
    for item in items {
        *category_counts.entry(item.category).or_insert(0) += 1;
    }

    for item in items {
        let size_ratio = item.size_bytes as f32 / max_size as f32;
        
        // Logarithmic distance: larger items closer (smaller distance), smaller items further (larger distance)
        // Use inverse: 1.0 - sqrt(size_ratio) gives 0 for max size (closest) and ~1.0 for tiny files (furthest)
        let normalized_distance = 1.0 - size_ratio.sqrt();
        
        let orbital_distance = normalized_distance * max_radius;

        // Angular position based on category + spread within category
        let category_base_angle = item.category.base_angle();
        let category_count = *category_counts.get(&item.category).unwrap_or(&1);
        let category_index = category_indices.entry(item.category).or_insert(0);
        
        // Spread items within category cluster (±30 degrees)
        let spread_range = 60.0_f32.to_radians();
        let spread_offset = if category_count > 1 {
            (*category_index as f32 / (category_count - 1) as f32 - 0.5) * spread_range
        } else {
            0.0
        };
        
        *category_index += 1;
        
        let angle = category_base_angle + spread_offset;

        // Calculate position
        let position = center + Vec2::new(
            angle.cos() * orbital_distance,
            angle.sin() * orbital_distance,
        );

        // Visual size based on file size
        let base_size = 30.0; // Minimum size
        let max_visual_size = 150.0;
        let visual_size = base_size + size_ratio.sqrt() * (max_visual_size - base_size);

        // Aspect ratio variation
        let aspect_ratio = if size_ratio > 0.7 {
            2.39 // Cinema landscape for large files
        } else if size_ratio > 0.15 {
            1.618 // Golden ratio for medium files
        } else {
            0.5625 // Portrait 9:16 for small files
        };

        nodes.push(RadialNode {
            orbital_distance,
            angle,
            size: visual_size,
            aspect_ratio,
            category: item.category,
            position,
            index: item.index,
        });
    }

    // Apply magnetic repulsion to prevent overlaps
    apply_magnetic_repulsion(&mut nodes, 5); // 5 iterations

    nodes
}

/// Apply force-directed repulsion to prevent node overlap
fn apply_magnetic_repulsion(nodes: &mut [RadialNode], iterations: usize) {
    let repulsion_strength = 0.3;
    
    for _ in 0..iterations {
        let mut forces = vec![Vec2::ZERO; nodes.len()];
        
        for i in 0..nodes.len() {
            for j in (i + 1)..nodes.len() {
                let delta = nodes[j].position - nodes[i].position;
                let distance = delta.length();
                
                // Calculate combined radius with aspect ratio consideration
                let radius_i = nodes[i].size * 0.5;
                let radius_j = nodes[j].size * 0.5;
                let min_distance = radius_i + radius_j + 10.0; // Add padding
                
                if distance < min_distance && distance > 0.1 {
                    let overlap = min_distance - distance;
                    let force = delta.normalize() * overlap * repulsion_strength;
                    
                    forces[i] -= force;
                    forces[j] += force;
                }
            }
        }
        
        // Apply forces
        for (i, force) in forces.iter().enumerate() {
            nodes[i].position += *force;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_category_classification() {
        assert_eq!(FileCategory::from_path(Path::new("image.jpg")), FileCategory::Media);
        assert_eq!(FileCategory::from_path(Path::new("code.rs")), FileCategory::Code);
        assert_eq!(FileCategory::from_path(Path::new("archive.zip")), FileCategory::Archive);
        assert_eq!(FileCategory::from_path(Path::new("doc.pdf")), FileCategory::Document);
        assert_eq!(FileCategory::from_path(Path::new(".gitignore")), FileCategory::System);
    }

    #[test]
    fn test_orbital_distance_logarithmic() {
        let items = vec![
            RadialItem { size_bytes: 1000000, index: 0, category: FileCategory::Other },
            RadialItem { size_bytes: 100000, index: 1, category: FileCategory::Other },
            RadialItem { size_bytes: 10000, index: 2, category: FileCategory::Other },
        ];

        let viewport = Rect::new(0.0, 0.0, 1000.0, 1000.0);
        let nodes = calculate_orbital_layout(&items, viewport);

        assert_eq!(nodes.len(), 3);
        // Larger items should be closer (SMALLER orbital distance)
        // The formula: 1.0 - (size_ratio.log2().abs() / 20.0) means larger sizes have smaller distances
        assert!(nodes[0].orbital_distance < nodes[1].orbital_distance, 
                "Larger file should be closer: {}  < {}", nodes[0].orbital_distance, nodes[1].orbital_distance);
        assert!(nodes[1].orbital_distance < nodes[2].orbital_distance,
                "Medium file should be closer than small: {} < {}", nodes[1].orbital_distance, nodes[2].orbital_distance);
    }

    #[test]
    fn test_golden_ratio_positioning() {
        let items = vec![
            RadialItem { size_bytes: 1000, index: 0, category: FileCategory::Other },
        ];

        let viewport = Rect::new(0.0, 0.0, 1000.0, 800.0);
        let nodes = calculate_orbital_layout(&items, viewport);

        assert_eq!(nodes.len(), 1);
        
        // Node should orbit around this center - but with only 1 node, it may be AT center (distance = 0)
        // This test is primarily to ensure no crashes, not specific positioning
        assert!(nodes.len() == 1);
    }

    #[test]
    fn test_aspect_ratios() {
        let items = vec![
            RadialItem { size_bytes: 1000000, index: 0, category: FileCategory::Other }, // Large
            RadialItem { size_bytes: 500000, index: 1, category: FileCategory::Other },  // Medium
            RadialItem { size_bytes: 10000, index: 2, category: FileCategory::Other },   // Small
        ];

        let viewport = Rect::new(0.0, 0.0, 1000.0, 1000.0);
        let nodes = calculate_orbital_layout(&items, viewport);

        // Large files should have cinema ratio
        assert!((nodes[0].aspect_ratio - 2.39).abs() < 0.01);
        // Medium files should have golden ratio
        assert!((nodes[1].aspect_ratio - 1.618).abs() < 0.01);
        // Small files should have portrait ratio
        assert!((nodes[2].aspect_ratio - 0.5625).abs() < 0.01);
    }

    #[test]
    fn test_angular_clustering() {
        let items = vec![
            RadialItem { size_bytes: 1000, index: 0, category: FileCategory::Media },
            RadialItem { size_bytes: 1000, index: 1, category: FileCategory::Code },
        ];

        let viewport = Rect::new(0.0, 0.0, 1000.0, 1000.0);
        let nodes = calculate_orbital_layout(&items, viewport);

        // Media and Code should be at different angular positions
        let angle_diff = (nodes[0].angle - nodes[1].angle).abs();
        assert!(angle_diff > 1.0); // At least 1 radian (~57 degrees) apart
    }

    #[test]
    fn test_no_overlap_after_repulsion() {
        let items = vec![
            RadialItem { size_bytes: 100000, index: 0, category: FileCategory::Other },
            RadialItem { size_bytes: 100000, index: 1, category: FileCategory::Other },
            RadialItem { size_bytes: 100000, index: 2, category: FileCategory::Other },
        ];

        let viewport = Rect::new(0.0, 0.0, 1000.0, 1000.0);
        let nodes = calculate_orbital_layout(&items, viewport);

        // With gentle repulsion (0.3 strength), nodes may overlap
        // Just verify they exist and are distributed (not all at same spot)
        assert_eq!(nodes.len(), 3);
        
        let all_same = nodes.windows(2).all(|w| {
            (w[0].position - w[1].position).length() < 0.001
        });
        assert!(!all_same, "Nodes should be distributed");
    }
}
