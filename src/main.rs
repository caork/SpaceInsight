use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

mod crawler;
mod tree;
mod treemap;

use crawler::{FileCrawler, ScanStats};
use tree::FileTree;
use treemap::{SquarifiedTreemap, Rect, TreemapItem, LayoutRect};

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("SpaceInsight - Disk Space Analyzer"),
        ..Default::default()
    };

    eframe::run_native(
        "SpaceInsight",
        options,
        Box::new(|_cc| Box::new(SpaceInsightApp::default())),
    )
}

#[derive(Default)]
struct SpaceInsightApp {
    scan_path: String,
    is_scanning: bool,
    scan_result: Arc<Mutex<Option<ScanResult>>>,
    layout: Vec<LayoutRect>,
    tree_items: Vec<TreeItem>,
    has_data: bool,
}

struct ScanResult {
    tree: FileTree,
    stats: ScanStats,
}

#[derive(Clone, Debug)]
struct TreeItem {
    path: PathBuf,
    name: String,
    size: u64,
}

impl SpaceInsightApp {
    fn start_scan(&mut self) {
        if self.is_scanning {
            return;
        }

        let path = if self.scan_path.is_empty() {
            ".".to_string()
        } else {
            self.scan_path.clone()
        };

        self.is_scanning = true;
        let scan_result = self.scan_result.clone();

        thread::spawn(move || {
            let mut crawler = FileCrawler::new();
            let (nodes, stats) = crawler.scan(&path);

            println!("Crawler found {} nodes", nodes.len());

            // Build tree
            let mut tree = FileTree::new(&path);
            
            // Sort paths to ensure parents are added before children
            let mut paths: Vec<_> = nodes.iter().map(|entry| entry.key().clone()).collect();
            paths.sort();

            for path_entry in paths {
                if let Some(node) = nodes.get(&path_entry) {
                    // Skip if this path is already in the tree (e.g., the root)
                    if tree.get_node(&node.path).is_none() {
                        tree.add_node(node.path.clone(), node.size, node.is_dir);
                    }
                }
            }

            tree.calculate_sizes();
            
            let root_id = tree.get_root();
            let arena = tree.get_arena();
            let child_count = root_id.children(arena).count();
            println!("Tree built! Root has {} children, total size: {}", child_count, tree.total_size());

            let result = ScanResult { tree, stats };
            
            *scan_result.lock().unwrap() = Some(result);
        });
    }

    fn update_layout(&mut self, container_rect: egui::Rect) {
        // Check if there's a scan result ready (non-blocking check)
        if let Ok(mut result_guard) = self.scan_result.try_lock() {
            if let Some(result) = result_guard.take() {
                self.is_scanning = false;
                
                // Get root node and collect immediate children
                let root = result.tree.get_root();
                let arena = result.tree.get_arena();
                
                self.tree_items.clear();
                
                for child_id in root.children(arena) {
                    if let Some(node) = arena.get(child_id) {
                        let data = node.get();
                        self.tree_items.push(TreeItem {
                            path: data.path.clone(),
                            name: data.name.clone(),
                            size: data.cumulative_size,
                        });
                    }
                }

                println!("Scan completed! Found {} items", self.tree_items.len());
                println!("Stats: {} files, {} dirs", result.stats.total_files, result.stats.total_dirs);
                
                self.has_data = !self.tree_items.is_empty();
            }
        }
        
        // Recalculate layout if we have data and container size changed
        if self.has_data && !self.tree_items.is_empty() {
            let items: Vec<TreemapItem> = self.tree_items
                .iter()
                .enumerate()
                .map(|(i, item)| TreemapItem {
                    size: item.size,
                    index: i,
                })
                .collect();

            let container = Rect::new(
                container_rect.min.x,
                container_rect.min.y,
                container_rect.width(),
                container_rect.height(),
            );

            self.layout = SquarifiedTreemap::layout(&items, container);
        }
    }

    fn format_size(size: u64) -> String {
        const KB: u64 = 1024;
        const MB: u64 = KB * 1024;
        const GB: u64 = MB * 1024;

        if size >= GB {
            format!("{:.2} GB", size as f64 / GB as f64)
        } else if size >= MB {
            format!("{:.2} MB", size as f64 / MB as f64)
        } else if size >= KB {
            format!("{:.2} KB", size as f64 / KB as f64)
        } else {
            format!("{} B", size)
        }
    }
}

impl eframe::App for SpaceInsightApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("SpaceInsight");
                ui.separator();
                
                ui.label("Path:");
                ui.text_edit_singleline(&mut self.scan_path);
                
                if ui.button("Scan").clicked() {
                    self.start_scan();
                }
                
                if self.is_scanning {
                    ui.spinner();
                    ui.label("Scanning...");
                }
                
                if self.has_data {
                    ui.label(format!("Items: {}", self.tree_items.len()));
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let available_rect = ui.available_rect_before_wrap();
            
            // Update layout if scan completed
            self.update_layout(available_rect);

            // Draw treemap
            let painter = ui.painter();
            
            for layout_rect in &self.layout {
                if layout_rect.index < self.tree_items.len() {
                    let item = &self.tree_items[layout_rect.index];
                    let rect = layout_rect.rect;
                    
                    let egui_rect = egui::Rect::from_min_size(
                        egui::pos2(rect.x, rect.y),
                        egui::vec2(rect.width, rect.height),
                    );

                    // Color based on size (hue varies with size)
                    let total_size: u64 = self.tree_items.iter().map(|i| i.size).sum();
                    let size_ratio = if total_size > 0 {
                        item.size as f32 / total_size as f32
                    } else {
                        0.0
                    };
                    
                    let hue = size_ratio * 0.6; // Range from 0 (red) to 0.6 (cyan)
                    let color = egui::Color32::from_rgb(
                        (255.0 * (1.0 - hue)) as u8,
                        (255.0 * hue) as u8,
                        128,
                    );

                    painter.rect_filled(egui_rect, 2.0, color);
                    painter.rect_stroke(egui_rect, 2.0, (1.0, egui::Color32::BLACK));

                    // Draw label if there's enough space
                    if rect.width > 50.0 && rect.height > 20.0 {
                        let text = format!("{}\n{}", item.name, Self::format_size(item.size));
                        painter.text(
                            egui_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            text,
                            egui::FontId::proportional(10.0),
                            egui::Color32::WHITE,
                        );
                    }
                }
            }

            // Request repaint if scanning
            if self.is_scanning {
                ctx.request_repaint();
            }
        });
    }
}
