use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

mod animation;
mod crawler;
mod expand_state;
mod render_tree;
mod tree;
mod treemap;

use animation::LayoutAnimator;
use crawler::{FileCrawler, ScanStats};
use expand_state::ExpansionState;
use render_tree::{build_render_tree, RenderNode, BORDER_VISUAL_WIDTH, HEADER_HEIGHT, SIDE_INSET};
use tree::FileTree;
use treemap::{Rect, SquarifiedTreemap, TreemapItem};

const TILE_GUTTER: f32 = 1.0;
const TILE_CORNER_MAX: f32 = 8.0;
const TILE_BORDER_WIDTH_DIR: f32 = 0.85;
const TILE_BORDER_WIDTH_FILE: f32 = 0.75;
const TILE_BORDER_WIDTH_AGG: f32 = 0.7;

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
            configure_custom_style(&cc.egui_ctx);
            Box::new(SpaceInsightApp::default())
        }),
    )
}

fn configure_custom_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();

    let mut visuals = egui::Visuals::dark();

    visuals.panel_fill = egui::Color32::from_rgba_unmultiplied(30, 41, 59, 240);
    visuals.window_fill = egui::Color32::from_rgba_unmultiplied(30, 41, 59, 230);

    visuals.window_stroke =
        egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 26));
    visuals.widgets.noninteractive.bg_stroke =
        egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 13));

    visuals.window_rounding = egui::Rounding::same(12.0);
    visuals.widgets.noninteractive.rounding = egui::Rounding::same(8.0);
    visuals.widgets.inactive.rounding = egui::Rounding::same(8.0);
    visuals.widgets.hovered.rounding = egui::Rounding::same(8.0);
    visuals.widgets.active.rounding = egui::Rounding::same(8.0);

    visuals.window_shadow = egui::epaint::Shadow::NONE;

    style.visuals = visuals;

    style.spacing.item_spacing = egui::vec2(12.0, 8.0);
    style.spacing.window_margin = egui::Margin::same(24.0);
    style.spacing.button_padding = egui::vec2(16.0, 8.0);

    ctx.set_style(style);
}

/// Actions resulting from user clicks in the treemap.
enum ClickAction {
    Expand(PathBuf),
    Deepen(PathBuf),
    Collapse(PathBuf),
    SelectFile(PathBuf),
}

struct SpaceInsightApp {
    scan_path: String,
    is_scanning: bool,
    scan_result: Arc<Mutex<Option<ScanResult>>>,
    has_data: bool,
    file_tree: Option<FileTree>,
    root_node_id: Option<indextree::NodeId>,
    expansion_state: ExpansionState,
    render_nodes: Vec<RenderNode>,
    hovered_path: Option<PathBuf>,
    selected_path: Option<PathBuf>,
    // Animation state (initial scan reveal only)
    animator: LayoutAnimator,
    last_frame_time: Option<Instant>,
    last_container_rect: Option<egui::Rect>,
    // Cached top-level items for animation
    top_level_items: Vec<TopLevelItem>,
}

#[derive(Clone)]
struct TopLevelItem {
    path: PathBuf,
    name: String,
    size: u64,
    is_dir: bool,
}

impl Default for SpaceInsightApp {
    fn default() -> Self {
        Self {
            scan_path: String::default(),
            is_scanning: false,
            scan_result: Arc::new(Mutex::new(None)),
            has_data: false,
            file_tree: None,
            root_node_id: None,
            expansion_state: ExpansionState::default(),
            render_nodes: Vec::new(),
            hovered_path: None,
            selected_path: None,
            animator: LayoutAnimator::default(),
            last_frame_time: None,
            last_container_rect: None,
            top_level_items: Vec::new(),
        }
    }
}

struct ScanResult {
    tree: FileTree,
    #[allow(dead_code)]
    stats: ScanStats,
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

            let mut tree = FileTree::new(&path);

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

            *scan_result.lock().unwrap() = Some(ScanResult { tree, stats });
        });
    }

    fn check_scan_result(&mut self, container_rect: egui::Rect) {
        let scan_complete = if let Ok(mut result_guard) = self.scan_result.try_lock() {
            if let Some(result) = result_guard.take() {
                self.is_scanning = false;
                let root = result.tree.get_root();
                self.root_node_id = Some(root);
                self.file_tree = Some(result.tree);
                self.expansion_state = ExpansionState::default();
                self.selected_path = None;
                true
            } else {
                false
            }
        } else {
            false
        };

        if scan_complete {
            self.has_data = true;
            self.populate_top_level_items();
            self.start_initial_animation(container_rect);
        }

        // Recompute render tree if container size changed
        if self.has_data && !self.animator.is_animating {
            let needs_recompute = match self.last_container_rect {
                Some(prev) => {
                    (prev.width() - container_rect.width()).abs() > 1.0
                        || (prev.height() - container_rect.height()).abs() > 1.0
                        || (prev.min.x - container_rect.min.x).abs() > 1.0
                        || (prev.min.y - container_rect.min.y).abs() > 1.0
                }
                None => true,
            };
            if needs_recompute {
                self.rebuild_render_tree(container_rect);
            }
        }
    }

    fn populate_top_level_items(&mut self) {
        self.top_level_items.clear();
        if let (Some(tree), Some(root_id)) = (&self.file_tree, self.root_node_id) {
            let arena = tree.get_arena();
            for child_id in root_id.children(arena) {
                if let Some(node) = arena.get(child_id) {
                    let data = node.get();
                    self.top_level_items.push(TopLevelItem {
                        path: data.path.clone(),
                        name: data.name.clone(),
                        size: data.cumulative_size,
                        is_dir: data.is_dir,
                    });
                }
            }
        }
    }

    fn start_initial_animation(&mut self, container_rect: egui::Rect) {
        self.last_container_rect = Some(container_rect);

        let padded = Self::padded_container(container_rect);

        let items: Vec<TreemapItem> = self
            .top_level_items
            .iter()
            .enumerate()
            .map(|(i, item)| TreemapItem {
                size: item.size,
                index: i,
            })
            .collect();

        let container = Rect::new(padded.min.x, padded.min.y, padded.width(), padded.height());
        let layout = SquarifiedTreemap::layout(&items, container);

        let targets: Vec<(f32, f32, f32, f32, usize)> = layout
            .iter()
            .map(|lr| (lr.rect.x, lr.rect.y, lr.rect.width, lr.rect.height, lr.index))
            .collect();

        let center = (padded.center().x, padded.center().y);
        let item_count = self.top_level_items.len();
        self.animator.start(&targets, center, item_count);
    }

    fn rebuild_render_tree(&mut self, container_rect: egui::Rect) {
        self.last_container_rect = Some(container_rect);

        if let (Some(tree), Some(root_id)) = (&self.file_tree, self.root_node_id) {
            let padded = Self::padded_container(container_rect);
            let container = Rect::new(padded.min.x, padded.min.y, padded.width(), padded.height());

            self.render_nodes = build_render_tree(
                tree,
                root_id,
                container,
                &self.expansion_state,
                render_tree::MAX_EXPAND_DEPTH,
            );
        }
    }

    fn padded_container(rect: egui::Rect) -> egui::Rect {
        egui::Rect::from_min_size(
            egui::pos2(rect.min.x + 4.0, rect.min.y + 4.0),
            egui::vec2((rect.width() - 8.0).max(1.0), (rect.height() - 8.0).max(1.0)),
        )
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

    fn get_temperature_color(size_ratio: f32, is_hovered: bool) -> egui::Color32 {
        let (r, g, b) = if size_ratio < 0.15 {
            let t = size_ratio / 0.15;
            (59.0 + 80.0 * t, 130.0 + 35.0 * t, 246.0)
        } else if size_ratio < 0.4 {
            (139.0, 92.0, 246.0)
        } else if size_ratio < 0.7 {
            let t = (size_ratio - 0.4) / 0.3;
            (245.0 + 6.0 * t, 158.0 + 33.0 * t, 11.0 + 25.0 * t)
        } else {
            let t = (size_ratio - 0.7) / 0.3;
            (239.0 + 9.0 * t, 68.0 + 45.0 * t, 68.0 + 45.0 * t)
        };

        let (r, g, b) = if is_hovered {
            (
                (r * 1.15).min(255.0),
                (g * 1.15).min(255.0),
                (b * 1.15).min(255.0),
            )
        } else {
            (r, g, b)
        };

        egui::Color32::from_rgb(r as u8, g as u8, b as u8)
    }

    fn draw_aurora_background(painter: &egui::Painter, rect: egui::Rect) {
        let top_color = egui::Color32::from_rgb(30, 41, 59);
        let bottom_color = egui::Color32::from_rgb(15, 118, 110);

        let mesh = Self::create_gradient_mesh(rect, top_color, bottom_color);
        painter.add(egui::Shape::Mesh(mesh));
    }

    fn create_gradient_mesh(
        rect: egui::Rect,
        top_color: egui::Color32,
        bottom_color: egui::Color32,
    ) -> egui::Mesh {
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

    /// Recursively render nodes and collect any click action.
    fn render_nodes_recursive(
        nodes: &[RenderNode],
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        total_size: u64,
        selected_path: &Option<PathBuf>,
        hovered_path: &mut Option<PathBuf>,
        min_label_area: f32,
    ) -> Option<ClickAction> {
        let mut action: Option<ClickAction> = None;

        for node in nodes {
            let is_expanded = node.content_rect.is_some();

            if is_expanded {
                // --- Expanded folder ---
                let outer = egui::Rect::from_min_size(
                    egui::pos2(node.outer_rect.x, node.outer_rect.y),
                    egui::vec2(node.outer_rect.width, node.outer_rect.height),
                );

                // Dimmed background fill with thin border
                painter.rect(
                    outer,
                    4.0,
                    egui::Color32::from_rgba_unmultiplied(20, 30, 45, 160),
                    egui::Stroke::new(
                        BORDER_VISUAL_WIDTH,
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 40),
                    ),
                );

                // Header bar background (subtle highlight)
                let header_rect = egui::Rect::from_min_size(
                    outer.left_top(),
                    egui::vec2(outer.width(), HEADER_HEIGHT),
                );
                painter.rect(
                    header_rect,
                    egui::Rounding { nw: 4.0, ne: 4.0, sw: 0.0, se: 0.0 },
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 15),
                    egui::Stroke::NONE,
                );

                // Folder name label in the header
                let label_y = outer.min.y + HEADER_HEIGHT * 0.5;
                painter.text(
                    egui::pos2(outer.min.x + SIDE_INSET + 4.0, label_y),
                    egui::Align2::LEFT_CENTER,
                    &node.name,
                    egui::FontId::proportional(11.0),
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 180),
                );

                // Collapse hit zone: only the header strip
                let header_id = egui::Id::new(node.stable_id).with("header");
                let header_response = ui.interact(header_rect, header_id, egui::Sense::click());
                if header_response.hovered() {
                    *hovered_path = Some(node.path.clone());
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if header_response.clicked() && action.is_none() {
                    action = Some(ClickAction::Collapse(node.path.clone()));
                }

                // Recursively render children inside content_rect
                if !node.children.is_empty() {
                    if let Some(sub_action) = Self::render_nodes_recursive(
                        &node.children,
                        ui,
                        painter,
                        total_size,
                        selected_path,
                        hovered_path,
                        min_label_area,
                    ) {
                        if action.is_none() {
                            action = Some(sub_action);
                        }
                    }
                }
            } else {
                // --- Collapsed / leaf node ---
                let gutter = TILE_GUTTER;
                let px = node.outer_rect.x + gutter;
                let py = node.outer_rect.y + gutter;
                let pw = (node.outer_rect.width - 2.0 * gutter).max(1.0);
                let ph = (node.outer_rect.height - 2.0 * gutter).max(1.0);

                let egui_rect = egui::Rect::from_min_size(
                    egui::pos2(px, py),
                    egui::vec2(pw, ph),
                );

                let node_id = egui::Id::new(node.stable_id);
                let response = ui.interact(egui_rect, node_id, egui::Sense::click());
                let is_hovered = response.hovered();

                if is_hovered {
                    *hovered_path = Some(node.path.clone());
                    if node.is_dir {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                }

                // Click handling (no click action for aggregate blocks)
                if action.is_none() && !node.is_aggregate {
                    if response.double_clicked() && node.is_dir {
                        action = Some(ClickAction::Deepen(node.path.clone()));
                    } else if response.clicked() {
                        if node.is_dir {
                            action = Some(ClickAction::Expand(node.path.clone()));
                        } else {
                            action = Some(ClickAction::SelectFile(node.path.clone()));
                        }
                    }
                }

                let corner_radius = (pw.min(ph) * 0.06).min(TILE_CORNER_MAX);

                if node.is_aggregate {
                    // --- Aggregate block: grey ---
                    let grey = if is_hovered {
                        egui::Color32::from_rgb(90, 95, 105)
                    } else {
                        egui::Color32::from_rgb(70, 75, 85)
                    };
                    painter.rect(egui_rect, corner_radius, grey, egui::Stroke::NONE);
                    painter.rect_stroke(
                        egui_rect,
                        corner_radius,
                        egui::Stroke::new(
                            TILE_BORDER_WIDTH_AGG,
                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 20),
                        ),
                    );

                    let area = pw * ph;
                    if area > min_label_area {
                        painter.text(
                            egui::pos2(egui_rect.center().x, egui_rect.center().y - 8.0),
                            egui::Align2::CENTER_CENTER,
                            &node.name,
                            egui::FontId::proportional(11.0),
                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 140),
                        );
                        painter.text(
                            egui::pos2(egui_rect.center().x, egui_rect.center().y + 8.0),
                            egui::Align2::CENTER_CENTER,
                            Self::format_size(node.size),
                            egui::FontId::proportional(10.0),
                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 100),
                        );
                    } else if is_hovered {
                        response.on_hover_text(format!(
                            "{} ({})",
                            node.name,
                            Self::format_size(node.size)
                        ));
                    }
                } else {
                    // --- Normal file/folder block ---
                    let size_ratio = if total_size > 0 {
                        node.size as f32 / total_size as f32
                    } else {
                        0.0
                    };

                    let base_color = Self::get_temperature_color(size_ratio, is_hovered);

                    // Shadow
                    let shadow_rect = egui_rect.translate(egui::vec2(0.0, 2.0));
                    painter.rect(
                        shadow_rect,
                        10.0,
                        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 25),
                        egui::Stroke::NONE,
                    );

                    // Main fill
                    painter.rect(egui_rect, corner_radius, base_color, egui::Stroke::NONE);

                    // Border
                    let border_stroke = if node.is_dir {
                        egui::Stroke::new(
                            TILE_BORDER_WIDTH_DIR,
                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 50),
                        )
                    } else {
                        egui::Stroke::new(
                            TILE_BORDER_WIDTH_FILE,
                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 30),
                        )
                    };
                    painter.rect_stroke(egui_rect, corner_radius, border_stroke);

                    // Selection glow
                    if selected_path.as_ref() == Some(&node.path) && !node.is_dir {
                        let glow_rect = egui_rect.expand(2.0);
                        painter.rect_stroke(
                            glow_rect,
                            corner_radius + 2.0,
                            egui::Stroke::new(
                                2.0,
                                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 100),
                            ),
                        );
                    }

                    // Labels
                    let area = pw * ph;
                    if area > min_label_area {
                        let dir_indicator = if node.is_dir { "+" } else { "" };

                        let (name_size, size_size) = if area > 10000.0 {
                            (14.0, 11.0)
                        } else {
                            (12.0, 10.0)
                        };

                        let name_text = if node.is_dir {
                            format!("{} {}", dir_indicator, node.name)
                        } else {
                            node.name.clone()
                        };

                        painter.text(
                            egui::pos2(egui_rect.center().x, egui_rect.center().y - 8.0),
                            egui::Align2::CENTER_CENTER,
                            name_text,
                            egui::FontId::proportional(name_size),
                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 255),
                        );

                        let size_text = Self::format_size(node.size);
                        painter.text(
                            egui::pos2(egui_rect.center().x, egui_rect.center().y + 8.0),
                            egui::Align2::CENTER_CENTER,
                            size_text,
                            egui::FontId::proportional(size_size),
                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 153),
                        );
                    } else if is_hovered {
                        let dir_indicator = if node.is_dir { "+" } else { "" };
                        let tooltip_text = format!(
                            "{} {} ({})",
                            dir_indicator,
                            node.name,
                            Self::format_size(node.size)
                        );
                        response.on_hover_text(tooltip_text);
                    }
                }
            }
        }

        action
    }

    /// Render the initial animation (top-level only, no expand until done).
    fn render_initial_animation(
        &self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
    ) {
        let anim_rects = self.animator.get_animated_rects();
        let total_size: u64 = self.top_level_items.iter().map(|i| i.size).sum();
        let min_label_area = self.animator.tier.min_label_area();

        for anim in anim_rects {
            if !anim.is_revealed {
                continue;
            }

            let item_index = anim.index;
            if item_index >= self.top_level_items.len() {
                continue;
            }

            let item = &self.top_level_items[item_index];
            let opacity = anim.current.opacity;

            let px = anim.current.x + TILE_GUTTER;
            let py = anim.current.y + TILE_GUTTER;
            let pw = (anim.current.w - 2.0 * TILE_GUTTER).max(1.0);
            let ph = (anim.current.h - 2.0 * TILE_GUTTER).max(1.0);

            let egui_rect =
                egui::Rect::from_min_size(egui::pos2(px, py), egui::vec2(pw, ph));

            let size_ratio = if total_size > 0 {
                item.size as f32 / total_size as f32
            } else {
                0.0
            };

            let base_color = Self::get_temperature_color(size_ratio, false);
            let alpha = (opacity * 255.0) as u8;
            let color = egui::Color32::from_rgba_unmultiplied(
                base_color.r(),
                base_color.g(),
                base_color.b(),
                alpha,
            );

            if opacity > 0.3 {
                let shadow_rect = egui_rect.translate(egui::vec2(0.0, 2.0));
                painter.rect(
                    shadow_rect,
                    10.0,
                    egui::Color32::from_rgba_unmultiplied(0, 0, 0, (25.0 * opacity) as u8),
                    egui::Stroke::NONE,
                );
            }

            let corner_radius = (pw.min(ph) * 0.06).min(TILE_CORNER_MAX);
            painter.rect(egui_rect, corner_radius, color, egui::Stroke::NONE);

            let border_stroke = if item.is_dir {
                egui::Stroke::new(
                    TILE_BORDER_WIDTH_DIR,
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, (50.0 * opacity) as u8),
                )
            } else {
                egui::Stroke::new(
                    TILE_BORDER_WIDTH_FILE,
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, (30.0 * opacity) as u8),
                )
            };
            painter.rect_stroke(egui_rect, corner_radius, border_stroke);

            let area = pw * ph;
            if area > min_label_area && opacity > 0.5 {
                let dir_indicator = if item.is_dir { "+" } else { "" };
                let (name_size, size_size) = if area > 10000.0 {
                    (14.0, 11.0)
                } else {
                    (12.0, 10.0)
                };

                let label_alpha = (opacity * 255.0) as u8;
                let name_text = if item.is_dir {
                    format!("{} {}", dir_indicator, item.name)
                } else {
                    item.name.clone()
                };
                painter.text(
                    egui::pos2(egui_rect.center().x, egui_rect.center().y - 8.0),
                    egui::Align2::CENTER_CENTER,
                    name_text,
                    egui::FontId::proportional(name_size),
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, label_alpha),
                );

                let size_text = Self::format_size(item.size);
                painter.text(
                    egui::pos2(egui_rect.center().x, egui_rect.center().y + 8.0),
                    egui::Align2::CENTER_CENTER,
                    size_text,
                    egui::FontId::proportional(size_size),
                    egui::Color32::from_rgba_unmultiplied(
                        255,
                        255,
                        255,
                        ((153.0 / 255.0) * opacity * 255.0) as u8,
                    ),
                );
            }

            // Hover during animation → snap to final
            let response = ui.interact(
                egui_rect,
                ui.id().with("anim").with(item_index),
                egui::Sense::hover(),
            );
            if response.hovered() {
                // We'll check after the loop
            }
        }
    }
}

impl eframe::App for SpaceInsightApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Calculate dt for animation
        let now = Instant::now();
        let dt = self
            .last_frame_time
            .map(|t| now.duration_since(t).as_secs_f32())
            .unwrap_or(1.0 / 60.0);
        self.last_frame_time = Some(now);

        // Update animation
        let still_animating = self.animator.update(dt);

        // If animation just finished, build the render tree
        let animation_just_finished = !still_animating && !self.animator.is_animating && self.has_data && self.render_nodes.is_empty();

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
                    if ui.button("Collapse All").clicked() {
                        self.expansion_state.collapse_all();
                        if let Some(rect) = self.last_container_rect {
                            self.rebuild_render_tree(rect);
                        }
                    }
                }
            });

            // Show root path and hovered item
            if self.has_data {
                ui.horizontal(|ui| {
                    ui.label(format!("Root: {}", if self.scan_path.is_empty() { "." } else { &self.scan_path }));
                    if let Some(ref hovered) = self.hovered_path {
                        ui.separator();
                        ui.label(format!("{}", hovered.display()));
                    }
                });
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let available_rect = ui.available_rect_before_wrap();

            self.check_scan_result(available_rect);

            if animation_just_finished {
                self.rebuild_render_tree(available_rect);
            }

            let painter = ui.painter().clone();

            Self::draw_aurora_background(&painter, available_rect);

            let mut new_hovered_path: Option<PathBuf> = None;

            if self.animator.is_animating {
                // During initial animation, render animated top-level blocks
                self.render_initial_animation(ui, &painter);

                // Check if user is hovering → snap animation
                let pointer = ui.input(|i| i.pointer.hover_pos());
                if let Some(_pos) = pointer {
                    // Check if pointer is within the treemap area
                    if available_rect.contains(_pos) && ui.input(|i| i.pointer.any_down()) {
                        self.animator.finish_immediately();
                        self.rebuild_render_tree(available_rect);
                    }
                }
            } else if !self.render_nodes.is_empty() {
                // Normal recursive rendering
                let total_size = if let Some(tree) = &self.file_tree {
                    tree.total_size()
                } else {
                    0
                };
                let min_label_area = self.animator.tier.min_label_area();

                let action = Self::render_nodes_recursive(
                    &self.render_nodes,
                    ui,
                    &painter,
                    total_size,
                    &self.selected_path,
                    &mut new_hovered_path,
                    min_label_area,
                );

                // Process click action
                if let Some(act) = action {
                    match act {
                        ClickAction::Expand(path) => {
                            self.expansion_state.expand(&path);
                            self.rebuild_render_tree(available_rect);
                        }
                        ClickAction::Deepen(path) => {
                            // Double-click: expand this folder AND its child folders
                            self.expansion_state.expand(&path);
                            if let Some(tree) = &self.file_tree {
                                if let Some(node_id) = tree.get_node(&path) {
                                    let arena = tree.get_arena();
                                    for child_id in node_id.children(arena) {
                                        if let Some(child_node) = arena.get(child_id) {
                                            let child_data = child_node.get();
                                            if child_data.is_dir {
                                                self.expansion_state.expand(&child_data.path);
                                            }
                                        }
                                    }
                                }
                            }
                            self.rebuild_render_tree(available_rect);
                        }
                        ClickAction::Collapse(path) => {
                            self.expansion_state.collapse_recursive(&path);
                            self.rebuild_render_tree(available_rect);
                        }
                        ClickAction::SelectFile(path) => {
                            self.selected_path = Some(path);
                        }
                    }
                }
            }

            self.hovered_path = new_hovered_path;

            if self.is_scanning || still_animating || self.animator.is_animating {
                ctx.request_repaint();
            }
        });
    }
}
