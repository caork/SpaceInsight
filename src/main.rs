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
        Box::new(|cc| {
            // Configure custom visuals for Apple-inspired aesthetic
            configure_custom_style(&cc.egui_ctx);
            Box::new(SpaceInsightApp::default())
        }),
    )
}

fn configure_custom_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    
    // Dark theme with deep slate background
    let mut visuals = egui::Visuals::dark();
    
    // Aurora gradient colors (will be rendered manually in background)
    visuals.panel_fill = egui::Color32::from_rgba_unmultiplied(30, 41, 59, 240);
    visuals.window_fill = egui::Color32::from_rgba_unmultiplied(30, 41, 59, 230);
    
    // Glass morphism - subtle borders
    visuals.window_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 26));
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 13));
    
    // Rounded corners (squircles)
    visuals.window_rounding = egui::Rounding::same(12.0);
    visuals.widgets.noninteractive.rounding = egui::Rounding::same(8.0);
    visuals.widgets.inactive.rounding = egui::Rounding::same(8.0);
    visuals.widgets.hovered.rounding = egui::Rounding::same(8.0);
    visuals.widgets.active.rounding = egui::Rounding::same(8.0);
    
    // Shadows for depth (using default shadow)
    visuals.window_shadow = egui::epaint::Shadow::NONE;
    
    style.visuals = visuals;
    
    // Typography - spacing and sizing
    style.spacing.item_spacing = egui::vec2(12.0, 8.0);
    style.spacing.window_margin = egui::Margin::same(24.0);
    style.spacing.button_padding = egui::vec2(16.0, 8.0);
    
    ctx.set_style(style);
}

struct SpaceInsightApp {
    scan_path: String,
    is_scanning: bool,
    scan_result: Arc<Mutex<Option<ScanResult>>>,
    layout: Vec<LayoutRect>,
    tree_items: Vec<TreeItem>,
    has_data: bool,
    // Navigation state
    file_tree: Option<FileTree>,
    current_view: Option<indextree::NodeId>,
    navigation_stack: Vec<PathBuf>,
    // Animation state
    hovered_index: Option<usize>,
}

impl Default for SpaceInsightApp {
    fn default() -> Self {
        Self {
            scan_path: String::default(),
            is_scanning: false,
            scan_result: Arc::new(Mutex::new(None)),
            layout: Vec::new(),
            tree_items: Vec::new(),
            has_data: false,
            file_tree: None,
            current_view: None,
            navigation_stack: Vec::new(),
            hovered_index: None,
        }
    }
}

struct ScanResult {
    tree: FileTree,
    #[allow(dead_code)]
    stats: ScanStats,
}

#[derive(Clone, Debug)]
struct TreeItem {
    path: PathBuf,
    name: String,
    size: u64,
    is_dir: bool,
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
        let scan_complete = if let Ok(mut result_guard) = self.scan_result.try_lock() {
            if let Some(result) = result_guard.take() {
                self.is_scanning = false;
                
                let root = result.tree.get_root();
                self.current_view = Some(root);
                self.file_tree = Some(result.tree);
                self.navigation_stack.clear();
                
                true
            } else {
                false
            }
        } else {
            false
        };
        
        if scan_complete {
            self.populate_current_view();
            println!("Scan completed! Found {} items", self.tree_items.len());
            self.has_data = !self.tree_items.is_empty();
        }
        
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

    fn populate_current_view(&mut self) {
        self.tree_items.clear();
        
        if let (Some(tree), Some(current_id)) = (&self.file_tree, self.current_view) {
            let arena = tree.get_arena();
            
            for child_id in current_id.children(arena) {
                if let Some(node) = arena.get(child_id) {
                    let data = node.get();
                    self.tree_items.push(TreeItem {
                        path: data.path.clone(),
                        name: data.name.clone(),
                        size: data.cumulative_size,
                        is_dir: data.is_dir,
                    });
                }
            }
        }
    }

    fn navigate_to(&mut self, path: &PathBuf) {
        if let Some(tree) = &self.file_tree {
            if let Some(node_id) = tree.get_node(path) {
                if let Some(current_id) = self.current_view {
                    if let Some(current_node) = tree.get_arena().get(current_id) {
                        self.navigation_stack.push(current_node.get().path.clone());
                    }
                }
                
                self.current_view = Some(node_id);
                self.populate_current_view();
            }
        }
    }

    fn navigate_back(&mut self) {
        if let Some(previous_path) = self.navigation_stack.pop() {
            if let Some(tree) = &self.file_tree {
                if let Some(node_id) = tree.get_node(&previous_path) {
                    self.current_view = Some(node_id);
                    self.populate_current_view();
                }
            }
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

    /// Temperature-based color palette: cool blues → warm amber → energetic coral
    fn get_temperature_color(size_ratio: f32, is_hovered: bool) -> egui::Color32 {
        let (r, g, b) = if size_ratio < 0.15 {
            // Cool blue range for small files
            let t = size_ratio / 0.15;
            (59.0 + 80.0 * t, 130.0 + 35.0 * t, 246.0)
        } else if size_ratio < 0.4 {
            // Purple range for medium files
            (139.0, 92.0, 246.0)
        } else if size_ratio < 0.7 {
            // Amber range for large files
            let t = (size_ratio - 0.4) / 0.3;
            (245.0 + 6.0 * t, 158.0 + 33.0 * t, 11.0 + 25.0 * t)
        } else {
            // Coral range for very large files
            let t = (size_ratio - 0.7) / 0.3;
            (239.0 + 9.0 * t, 68.0 + 45.0 * t, 68.0 + 45.0 * t)
        };

        let (r, g, b) = if is_hovered {
            ((r * 1.15).min(255.0), (g * 1.15).min(255.0), (b * 1.15).min(255.0))
        } else {
            (r, g, b)
        };

        egui::Color32::from_rgb(r as u8, g as u8, b as u8)
    }

    /// Draw aurora gradient background that shifts with folder depth
    fn draw_aurora_background(&self, painter: &egui::Painter, rect: egui::Rect) {
        let depth = self.navigation_stack.len() as f32;
        let depth_factor = (depth * 0.1).min(0.3);
        
        let top_color = egui::Color32::from_rgb(
            (30.0 - depth_factor * 10.0) as u8,
            (41.0 + depth_factor * 35.0) as u8,
            (59.0 + depth_factor * 59.0) as u8,
        );
        
        let bottom_color = egui::Color32::from_rgb(
            (15.0 - depth_factor * 5.0) as u8,
            (118.0 - depth_factor * 20.0) as u8,
            (110.0 + depth_factor * 8.0) as u8,
        );
        
        let mesh = Self::create_gradient_mesh(rect, top_color, bottom_color);
        painter.add(egui::Shape::Mesh(mesh));
    }
    
    fn create_gradient_mesh(rect: egui::Rect, top_color: egui::Color32, bottom_color: egui::Color32) -> egui::Mesh {
        let mut mesh = egui::Mesh::default();
        
        mesh.vertices.push(egui::epaint::Vertex {
            pos: rect.left_top(),
            uv: egui::pos2(0.0, 0.0),
            color: top_color,
        });
        mesh.vertices.push(egui::epaint::Vertex {
            pos: rect.right_top(),
            uv: egui::pos2(1.0, 0.0),
            color: top_color,
        });
        mesh.vertices.push(egui::epaint::Vertex {
            pos: rect.right_bottom(),
            uv: egui::pos2(1.0, 1.0),
            color: bottom_color,
        });
        mesh.vertices.push(egui::epaint::Vertex {
            pos: rect.left_bottom(),
            uv: egui::pos2(0.0, 1.0),
            color: bottom_color,
        });
        
        mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        
        mesh
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
            
            // Breadcrumb navigation
            if self.has_data && self.file_tree.is_some() {
                ui.horizontal(|ui| {
                    if !self.navigation_stack.is_empty() {
                        if ui.button("⬅ Back").clicked() {
                            self.navigate_back();
                        }
                    }
                    
                    ui.separator();
                    
                    if let (Some(tree), Some(current_id)) = (&self.file_tree, self.current_view) {
                        if let Some(node) = tree.get_arena().get(current_id) {
                            let current_path = &node.get().path;
                            ui.label("📁");
                            ui.label(format!("{}", current_path.display()));
                        }
                    }
                });
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let available_rect = ui.available_rect_before_wrap();
            
            self.update_layout(available_rect);

            let painter = ui.painter();
            
            // Draw aurora gradient background
            self.draw_aurora_background(painter, available_rect);
            
            let mut clicked_path: Option<PathBuf> = None;
            let mut new_hovered_index: Option<usize> = None;
            let total_size: u64 = self.tree_items.iter().map(|i| i.size).sum();
            
            for (idx, layout_rect) in self.layout.iter().enumerate() {
                if layout_rect.index < self.tree_items.len() {
                    let item = &self.tree_items[layout_rect.index];
                    let rect = layout_rect.rect;
                    
                    // Add padding between rectangles for breathing room
                    let padded_rect = Rect::new(
                        rect.x + 2.0,
                        rect.y + 2.0,
                        (rect.width - 4.0).max(1.0),
                        (rect.height - 4.0).max(1.0),
                    );
                    
                    let egui_rect = egui::Rect::from_min_size(
                        egui::pos2(padded_rect.x, padded_rect.y),
                        egui::vec2(padded_rect.width, padded_rect.height),
                    );

                    let response = ui.interact(egui_rect, ui.id().with(idx), egui::Sense::click());
                    let is_hovered = response.hovered();
                    
                    if is_hovered {
                        new_hovered_index = Some(idx);
                        if item.is_dir {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                    }
                    
                    if response.clicked() && item.is_dir {
                        clicked_path = Some(item.path.clone());
                    }

                    let size_ratio = if total_size > 0 {
                        item.size as f32 / total_size as f32
                    } else {
                        0.0
                    };
                    
                    let color = Self::get_temperature_color(size_ratio, is_hovered);
                    
                    // Draw shadow for depth
                    let shadow_rect = egui_rect.translate(egui::vec2(0.0, 2.0));
                    painter.rect(
                        shadow_rect,
                        10.0,
                        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 25),
                        egui::Stroke::NONE,
                    );

                    // Draw main rectangle with rounded corners (squircles)
                    let corner_radius = (padded_rect.width.min(padded_rect.height) * 0.08).min(12.0);
                    painter.rect(
                        egui_rect,
                        corner_radius,
                        color,
                        egui::Stroke::NONE,
                    );
                    
                    // Glass morphism border
                    painter.rect_stroke(
                        egui_rect,
                        corner_radius,
                        egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 30)),
                    );

                    // Smart label placement - only show if enough space
                    let min_label_area = 2500.0;
                    if padded_rect.width * padded_rect.height > min_label_area {
                        let dir_indicator = if item.is_dir { "📁" } else { "📄" };
                        
                        let (name_size, size_size) = if padded_rect.width * padded_rect.height > 10000.0 {
                            (14.0, 11.0)
                        } else {
                            (12.0, 10.0)
                        };
                        
                        let name_text = format!("{} {}", dir_indicator, item.name);
                        painter.text(
                            egui::pos2(egui_rect.center().x, egui_rect.center().y - 8.0),
                            egui::Align2::CENTER_CENTER,
                            name_text,
                            egui::FontId::proportional(name_size),
                            egui::Color32::WHITE,
                        );
                        
                        let size_text = Self::format_size(item.size);
                        painter.text(
                            egui::pos2(egui_rect.center().x, egui_rect.center().y + 8.0),
                            egui::Align2::CENTER_CENTER,
                            size_text,
                            egui::FontId::proportional(size_size),
                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 153),
                        );
                    }
                }
            }
            
            self.hovered_index = new_hovered_index;
            
            if let Some(path) = clicked_path {
                self.navigate_to(&path);
            }

            if self.is_scanning {
                ctx.request_repaint();
            }
        });
    }
}
