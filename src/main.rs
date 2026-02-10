use eframe::egui;
use notify::{event::ModifyKind, Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use spaceinsight::animation::LayoutAnimator;
use spaceinsight::crawler::{FileCrawler, ScanPhase, ScanProgress, ScanStats, ScanTopLevelPreview};
use spaceinsight::expand_state::ExpansionState;
use spaceinsight::render_tree::{
    build_render_tree, RenderNode, BORDER_VISUAL_WIDTH, HEADER_HEIGHT, SIDE_INSET,
};
use spaceinsight::tree::FileTree;
use spaceinsight::treemap::{Rect, SquarifiedTreemap, TreemapItem};

const TILE_GUTTER: f32 = 1.0;
const TILE_CORNER_MAX: f32 = 8.0;
const TILE_BORDER_WIDTH_DIR: f32 = 0.85;
const TILE_BORDER_WIDTH_FILE: f32 = 0.75;
const TILE_BORDER_WIDTH_AGG: f32 = 0.7;
const AUTO_JUMP_SENSITIVITY: f32 = 0.0;
const AUTO_JUMP_MIN_USEFUL_AREA: f32 = 400.0;
const AUTO_JUMP_MIN_AREA_PCT: f32 = 0.005;
const AUTO_JUMP_VISIBLE_CAP: usize = 12;
const LAYOUT_TRANSITION_DURATION: f32 = 0.22;

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
    OpenPath(PathBuf),
    OpenInFileManager { path: PathBuf, is_dir: bool },
}

enum ScanEvent {
    Progress(ScanProgress),
    Completed(ScanResult),
}

struct LayoutTransition {
    from_rects: HashMap<u64, Rect>,
    target_nodes: Vec<RenderNode>,
    elapsed: f32,
}

struct SpaceInsightApp {
    scan_path: String,
    is_scanning: bool,
    scan_rx: Option<Receiver<ScanEvent>>,
    scan_progress: Option<ScanProgress>,
    scan_preview_items: Vec<TopLevelItem>,
    watcher: Option<RecommendedWatcher>,
    watcher_rx: Option<Receiver<notify::Result<Event>>>,
    watched_root: Option<PathBuf>,
    has_data: bool,
    file_tree: Option<FileTree>,
    root_node_id: Option<indextree::NodeId>,
    view_root_path: Option<PathBuf>,
    expansion_state: ExpansionState,
    render_nodes: Vec<RenderNode>,
    layout_transition: Option<LayoutTransition>,
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
    name: String,
    size: u64,
    is_dir: bool,
}

impl Default for SpaceInsightApp {
    fn default() -> Self {
        Self {
            scan_path: String::default(),
            is_scanning: false,
            scan_rx: None,
            scan_progress: None,
            scan_preview_items: Vec::new(),
            watcher: None,
            watcher_rx: None,
            watched_root: None,
            has_data: false,
            file_tree: None,
            root_node_id: None,
            view_root_path: None,
            expansion_state: ExpansionState::default(),
            render_nodes: Vec::new(),
            layout_transition: None,
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
        self.start_scan_internal();
    }

    fn start_scan_internal(&mut self) {
        if self.is_scanning {
            return;
        }

        let path = if self.scan_path.is_empty() {
            ".".to_string()
        } else {
            self.scan_path.clone()
        };

        self.scan_path = path.clone();
        self.is_scanning = true;
        self.scan_preview_items.clear();
        self.scan_progress = Some(ScanProgress {
            phase: ScanPhase::Discovering,
            discovered_entries: 0,
            processed_entries: 0,
            total_entries: None,
            total_files: 0,
            total_dirs: 0,
            total_size: 0,
            top_level_preview: Vec::new(),
        });

        let (tx, rx) = mpsc::channel::<ScanEvent>();
        self.scan_rx = Some(rx);

        thread::spawn(move || {
            let mut crawler = FileCrawler::new();
            let progress_tx = tx.clone();
            let reporter = Arc::new(move |progress: ScanProgress| {
                let _ = progress_tx.send(ScanEvent::Progress(progress));
            });

            let (nodes, stats) = crawler.scan_with_progress(&path, Some(reporter));

            let mut tree = FileTree::new(&path);

            for node in nodes {
                tree.upsert_node(node.path, node.size, node.is_dir);
            }

            tree.calculate_sizes();

            let _ = tx.send(ScanEvent::Completed(ScanResult {
                tree,
                stats,
            }));
        });
    }

    fn check_scan_result(&mut self, container_rect: egui::Rect) {
        let mut completed_scan: Option<ScanResult> = None;
        let mut rx_disconnected = false;

        if let Some(rx) = self.scan_rx.as_ref() {
            loop {
                match rx.try_recv() {
                    Ok(ScanEvent::Progress(progress)) => {
                        self.scan_preview_items = Self::preview_to_top_level_items(&progress.top_level_preview);
                        self.scan_progress = Some(progress);
                    }
                    Ok(ScanEvent::Completed(result)) => {
                        completed_scan = Some(result);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        rx_disconnected = true;
                        break;
                    }
                }
            }
        }

        if rx_disconnected {
            self.scan_rx = None;
            self.is_scanning = false;
        }

        if let Some(result) = completed_scan {
            self.scan_rx = None;
            self.is_scanning = false;
            self.scan_progress = None;
            self.scan_preview_items.clear();
            self.apply_scan_result(result, container_rect);
        }

        // Recompute render tree if container size changed
        if self.has_data && !self.animator.is_animating && self.layout_transition.is_none() {
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

    fn apply_scan_result(&mut self, result: ScanResult, container_rect: egui::Rect) {
        self.has_data = true;
        let root = result.tree.get_root();
        self.root_node_id = Some(root);
        self.file_tree = Some(result.tree);

        self.view_root_path = None;
        self.expansion_state = ExpansionState::default();
        self.selected_path = None;
        self.hovered_path = None;
        self.render_nodes.clear();
        self.layout_transition = None;
        self.populate_top_level_items();
        self.start_initial_animation(container_rect);
        self.install_file_watcher();
    }

    fn install_file_watcher(&mut self) {
        let Some(tree) = self.file_tree.as_ref() else {
            return;
        };
        let Some(root_path) = tree.root_path().map(|p| p.to_path_buf()) else {
            return;
        };

        if self.watched_root.as_ref() == Some(&root_path) && self.watcher.is_some() {
            return;
        }

        let (tx, rx) = mpsc::channel();
        let mut watcher = match RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            Config::default(),
        ) {
            Ok(w) => w,
            Err(_) => {
                self.watcher = None;
                self.watcher_rx = None;
                self.watched_root = None;
                return;
            }
        };

        if watcher.watch(&root_path, RecursiveMode::Recursive).is_ok() {
            self.watcher = Some(watcher);
            self.watcher_rx = Some(rx);
            self.watched_root = Some(root_path);
        } else {
            self.watcher = None;
            self.watcher_rx = None;
            self.watched_root = None;
        }
    }

    fn consume_file_events(&mut self, container_rect: egui::Rect) {
        if self.is_scanning {
            return;
        }

        let mut disconnected = false;
        let mut changed = false;
        let mut pending_events = Vec::new();

        if let Some(rx) = self.watcher_rx.as_ref() {
            loop {
                match rx.try_recv() {
                    Ok(Ok(event)) => {
                        pending_events.push(event);
                    }
                    Ok(Err(_)) => {}
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }

        for event in pending_events {
            changed |= self.apply_fs_event(event);
        }

        if disconnected {
            self.watcher = None;
            self.watcher_rx = None;
            self.watched_root = None;
        }

        if changed {
            if let Some(tree) = self.file_tree.as_mut() {
                tree.calculate_sizes();
            }
            self.prune_view_state_after_diff();
            self.populate_top_level_items();
            self.rebuild_render_tree(container_rect);
        }
    }

    fn apply_fs_event(&mut self, event: Event) -> bool {
        let mut changed = false;

        match event.kind {
            EventKind::Create(_) => {
                for path in event.paths {
                    if let Some(meta) = std::fs::metadata(&path).ok() {
                        changed |= self.apply_upsert_path(path, &meta);
                    }
                }
            }
            EventKind::Remove(_) => {
                for path in event.paths {
                    changed |= self.apply_remove_path(path.as_path());
                }
            }
            EventKind::Modify(ModifyKind::Name(_)) => {
                if event.paths.len() >= 2 {
                    let from = &event.paths[0];
                    let to = &event.paths[1];
                    changed |= self.apply_remove_path(from.as_path());
                    if let Some(meta) = std::fs::metadata(to).ok() {
                        changed |= self.apply_upsert_path(to.clone(), &meta);
                    }
                } else {
                    for path in event.paths {
                        if let Some(meta) = std::fs::metadata(&path).ok() {
                            changed |= self.apply_upsert_path(path, &meta);
                        } else {
                            changed |= self.apply_remove_path(path.as_path());
                        }
                    }
                }
            }
            EventKind::Modify(_) => {
                for path in event.paths {
                    if let Some(meta) = std::fs::metadata(&path).ok() {
                        changed |= self.apply_upsert_path(path, &meta);
                    } else {
                        changed |= self.apply_remove_path(path.as_path());
                    }
                }
            }
            _ => {}
        }

        changed
    }

    fn apply_upsert_path(&mut self, path: PathBuf, meta: &std::fs::Metadata) -> bool {
        let Some(tree) = self.file_tree.as_mut() else {
            return false;
        };

        let Some(root) = tree.root_path() else {
            return false;
        };
        if !path.starts_with(root) {
            return false;
        }

        let size = if meta.is_file() { meta.len() } else { 0 };
        let is_dir = meta.is_dir();

        tree.upsert_node(path, size, is_dir);
        true
    }

    fn apply_remove_path(&mut self, path: &Path) -> bool {
        let Some(tree) = self.file_tree.as_mut() else {
            return false;
        };

        let Some(root) = tree.root_path() else {
            return false;
        };
        if !path.starts_with(root) {
            return false;
        }

        tree.remove_path_recursive(path)
    }

    fn prune_view_state_after_diff(&mut self) {
        let Some(tree) = self.file_tree.as_ref() else {
            return;
        };

        self.expansion_state
            .retain_paths(|path| tree.get_node(path).is_some());

        if let Some(path) = self.selected_path.clone() {
            if tree.get_node(&path).is_none() {
                self.selected_path = None;
            }
        }

        if let Some(path) = self.hovered_path.clone() {
            if tree.get_node(&path).is_none() {
                self.hovered_path = None;
            }
        }

        if let Some(path) = self.view_root_path.clone() {
            if tree.get_node(&path).is_none() {
                self.view_root_path = None;
            }
        }
    }

    fn preview_to_top_level_items(preview: &[ScanTopLevelPreview]) -> Vec<TopLevelItem> {
        let mut items: Vec<TopLevelItem> = preview
            .iter()
            .map(|item| TopLevelItem {
                name: item.name.clone(),
                size: item.size,
                is_dir: item.is_dir,
            })
            .collect();

        items.sort_by(|a, b| b.size.cmp(&a.size));
        items
    }

    fn render_live_scan_preview(
        &self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        available_rect: egui::Rect,
    ) {
        if self.scan_preview_items.is_empty() {
            return;
        }

        let items: Vec<TreemapItem> = self
            .scan_preview_items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                if item.size == 0 {
                    None
                } else {
                    Some(TreemapItem { size: item.size, index })
                }
            })
            .collect();

        if items.is_empty() {
            return;
        }

        let padded = Self::padded_container(available_rect);
        let container = Rect::new(padded.min.x, padded.min.y, padded.width(), padded.height());
        let layout = SquarifiedTreemap::layout(&items, container);
        let total_size: u64 = self.scan_preview_items.iter().map(|item| item.size).sum();

        for layout_rect in layout {
            let Some(item) = self.scan_preview_items.get(layout_rect.index) else {
                continue;
            };

            let px = layout_rect.rect.x + TILE_GUTTER;
            let py = layout_rect.rect.y + TILE_GUTTER;
            let pw = (layout_rect.rect.width - 2.0 * TILE_GUTTER).max(1.0);
            let ph = (layout_rect.rect.height - 2.0 * TILE_GUTTER).max(1.0);

            let egui_rect = egui::Rect::from_min_size(egui::pos2(px, py), egui::vec2(pw, ph));

            let size_ratio = if total_size > 0 {
                item.size as f32 / total_size as f32
            } else {
                0.0
            };

            let color = Self::get_temperature_color(size_ratio, false);
            let corner_radius = (pw.min(ph) * 0.06).min(TILE_CORNER_MAX);

            painter.rect(
                egui_rect,
                corner_radius,
                color,
                egui::Stroke::new(
                    if item.is_dir {
                        TILE_BORDER_WIDTH_DIR
                    } else {
                        TILE_BORDER_WIDTH_FILE
                    },
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 60),
                ),
            );

            let area = pw * ph;
            if area > 2500.0 {
                let label = if item.is_dir {
                    format!("+ {}", item.name)
                } else {
                    item.name.clone()
                };
                let _ = Self::draw_centered_two_line_label(
                    painter,
                    egui_rect,
                    &label,
                    12.0,
                    &Self::format_size(item.size),
                    10.0,
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 230),
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 170),
                );
            }

            let response = ui.interact(
                egui_rect,
                ui.id().with("scan_preview").with(layout_rect.index),
                egui::Sense::hover(),
            );
            if response.hovered() {
                response.on_hover_text(format!("{} ({})", item.name, Self::format_size(item.size)));
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

        if let (Some(tree), Some(root_id)) = (&self.file_tree, self.active_root_node_id()) {
            let padded = Self::padded_container(container_rect);
            let container = Rect::new(padded.min.x, padded.min.y, padded.width(), padded.height());

            let next_nodes = build_render_tree(
                tree,
                root_id,
                container,
                &self.expansion_state,
                usize::MAX,
            );

            if self.render_nodes.is_empty() || next_nodes.is_empty() {
                self.layout_transition = None;
                self.render_nodes = next_nodes;
                return;
            }

            let from_rects = self.collect_current_rects();
            self.layout_transition = Some(LayoutTransition {
                from_rects,
                target_nodes: next_nodes.clone(),
                elapsed: 0.0,
            });
            self.render_nodes = next_nodes;
        }
    }

    fn collect_current_rects(&self) -> HashMap<u64, Rect> {
        let mut rects = HashMap::new();
        if let Some(transition) = &self.layout_transition {
            let mut nodes = transition.target_nodes.clone();
            let t = (transition.elapsed / LAYOUT_TRANSITION_DURATION).clamp(0.0, 1.0);
            Self::apply_transition_to_nodes(&mut nodes, &transition.from_rects, t);
            Self::collect_rects_recursive(&nodes, &mut rects);
        } else {
            Self::collect_rects_recursive(&self.render_nodes, &mut rects);
        }
        rects
    }

    fn collect_rects_recursive(nodes: &[RenderNode], out: &mut HashMap<u64, Rect>) {
        for node in nodes {
            out.insert(node.stable_id, node.outer_rect);
            if !node.children.is_empty() {
                Self::collect_rects_recursive(&node.children, out);
            }
        }
    }

    fn current_display_nodes(&self) -> Vec<RenderNode> {
        if let Some(transition) = &self.layout_transition {
            let mut nodes = transition.target_nodes.clone();
            let t = (transition.elapsed / LAYOUT_TRANSITION_DURATION).clamp(0.0, 1.0);
            Self::apply_transition_to_nodes(&mut nodes, &transition.from_rects, t);
            nodes
        } else {
            self.render_nodes.clone()
        }
    }

    fn apply_transition_to_nodes(nodes: &mut [RenderNode], from_rects: &HashMap<u64, Rect>, t: f32) {
        for node in nodes {
            if let Some(from) = from_rects.get(&node.stable_id) {
                node.outer_rect = Self::lerp_rect(*from, node.outer_rect, t);
            }
            if !node.children.is_empty() {
                Self::apply_transition_to_nodes(&mut node.children, from_rects, t);
            }
        }
    }

    fn lerp_rect(from: Rect, to: Rect, t: f32) -> Rect {
        Rect {
            x: from.x + (to.x - from.x) * t,
            y: from.y + (to.y - from.y) * t,
            width: from.width + (to.width - from.width) * t,
            height: from.height + (to.height - from.height) * t,
        }
    }

    fn update_layout_transition(&mut self, dt: f32) -> bool {
        if let Some(transition) = self.layout_transition.as_mut() {
            transition.elapsed += dt;
            if transition.elapsed >= LAYOUT_TRANSITION_DURATION {
                self.layout_transition = None;
                return false;
            }
            return true;
        }
        false
    }

    fn active_root_node_id(&self) -> Option<indextree::NodeId> {
        if let (Some(tree), Some(global_root_id)) = (&self.file_tree, self.root_node_id) {
            if let Some(path) = &self.view_root_path {
                tree.get_node(path).or(Some(global_root_id))
            } else {
                Some(global_root_id)
            }
        } else {
            None
        }
    }

    fn current_root_label(&self) -> String {
        if let (Some(tree), Some(active_root_id)) = (&self.file_tree, self.active_root_node_id()) {
            if let Some(node) = tree.get_arena().get(active_root_id) {
                return node.get().path.display().to_string();
            }
        }

        if self.scan_path.is_empty() {
            ".".to_string()
        } else {
            self.scan_path.clone()
        }
    }

    fn current_root_size(&self) -> Option<u64> {
        let (Some(tree), Some(active_root_id)) = (&self.file_tree, self.active_root_node_id()) else {
            return None;
        };

        tree.get_arena()
            .get(active_root_id)
            .map(|node| node.get().cumulative_size)
    }

    fn step_out_view_root(&mut self) {
        let (Some(tree), Some(global_root_id), Some(current_view_root)) =
            (&self.file_tree, self.root_node_id, self.view_root_path.clone())
        else {
            return;
        };

        let arena = tree.get_arena();
        let Some(global_root_path) = arena.get(global_root_id).map(|n| n.get().path.clone()) else {
            self.view_root_path = None;
            return;
        };

        if current_view_root == global_root_path {
            self.view_root_path = None;
            return;
        }

        if let Some(parent) = current_view_root.parent() {
            let parent_buf = parent.to_path_buf();
            if parent_buf == global_root_path {
                self.view_root_path = None;
            } else if tree.get_node(&parent_buf).is_some() {
                self.view_root_path = Some(parent_buf);
            } else {
                self.view_root_path = None;
            }
        } else {
            self.view_root_path = None;
        }
    }

    fn find_render_node_by_path<'a>(nodes: &'a [RenderNode], target: &Path) -> Option<&'a RenderNode> {
        for node in nodes {
            if node.path == target {
                return Some(node);
            }
            if let Some(found) = Self::find_render_node_by_path(&node.children, target) {
                return Some(found);
            }
        }
        None
    }

    fn should_zoom_into_folder(&self, path: &Path, viewport_rect: egui::Rect) -> bool {
        let Some(node) = Self::find_render_node_by_path(&self.render_nodes, path) else {
            return false;
        };

        let Some(tree) = self.file_tree.as_ref() else {
            return false;
        };
        let Some(node_id) = tree.get_node(path) else {
            return false;
        };

        let arena = tree.get_arena();
        let mut child_count = 0usize;
        let mut dir_children = 0usize;

        for child_id in node_id.children(arena) {
            child_count += 1;
            if let Some(child) = arena.get(child_id) {
                if child.get().is_dir {
                    dir_children += 1;
                }
            }
        }

        if child_count == 0 {
            return false;
        }

        if child_count <= 2 {
            return false;
        }

        let content_width = (node.outer_rect.width - 2.0 * SIDE_INSET).max(1.0);
        let content_height = (node.outer_rect.height - HEADER_HEIGHT - SIDE_INSET).max(1.0);
        let content_area = (content_width * content_height).max(1.0);

        let min_area = AUTO_JUMP_MIN_USEFUL_AREA.max(content_area * AUTO_JUMP_MIN_AREA_PCT);

        let mut total_size: u64 = 0;
        let mut renderable_count = 0usize;
        let mut renderable_area_sum = 0.0f32;

        let children: Vec<_> = node_id.children(arena).collect();
        for child_id in &children {
            if let Some(child) = arena.get(*child_id) {
                total_size += child.get().cumulative_size;
            }
        }

        if total_size == 0 {
            return false;
        }

        for child_id in children {
            if let Some(child) = arena.get(child_id) {
                let size = child.get().cumulative_size;
                let estimated_area = size as f32 / total_size as f32 * content_area;
                if estimated_area >= min_area {
                    renderable_count += 1;
                    renderable_area_sum += estimated_area;
                }
            }
        }

        let hidden_count = child_count.saturating_sub(renderable_count);
        let hidden_ratio = hidden_count as f32 / child_count as f32;
        let avg_renderable_area = if renderable_count > 0 {
            renderable_area_sum / renderable_count as f32
        } else {
            0.0
        };

        let sensitivity = AUTO_JUMP_SENSITIVITY;
        let hidden_threshold = 0.96 - 0.3 * sensitivity;
        let pressure_threshold = 2.2 - 0.9 * sensitivity;
        let visible_min_needed = if sensitivity < 0.35 { 2 } else { 1 };
        let area_pressure = if avg_renderable_area > 0.0 {
            min_area / avg_renderable_area
        } else {
            f32::INFINITY
        };

        // If expanded region is fairly large and children are limited, render in-place.
        let viewport_area = (viewport_rect.width() * viewport_rect.height()).max(1.0);
        let area_ratio = (content_area / viewport_area).clamp(0.0005, 1.0);
        if area_ratio > 0.35 && child_count <= 16 {
            return false;
        }

        // Jump only when in-place rendering quality is expected to be very poor.
        let too_many_hidden = hidden_ratio >= hidden_threshold && child_count >= 14;
        let too_few_visible = renderable_count <= visible_min_needed && child_count >= 16;
        let overloaded_visible = renderable_count > AUTO_JUMP_VISIBLE_CAP
            && hidden_ratio > hidden_threshold * 0.95
            && child_count >= 24;
        let pressure_too_high = area_pressure > pressure_threshold && child_count >= 20;
        let region_too_small_for_density =
            content_width < 130.0 && content_height < 95.0 && child_count >= 18 && dir_children >= 5;

        too_many_hidden
            || too_few_visible
            || overloaded_visible
            || pressure_too_high
            || region_too_small_for_density
    }

    fn pick_folder_path() -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            let output = Command::new("osascript")
                .args([
                    "-e",
                    "POSIX path of (choose folder with prompt \"Select folder to scan\")",
                ])
                .output()
                .ok()?;

            if !output.status.success() {
                return None;
            }

            let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if selected.is_empty() {
                None
            } else {
                Some(PathBuf::from(selected))
            }
        }

        #[cfg(target_os = "windows")]
        {
            let script = "Add-Type -AssemblyName System.Windows.Forms; $dlg = New-Object System.Windows.Forms.FolderBrowserDialog; $dlg.ShowNewFolderButton = $false; if ($dlg.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { Write-Output $dlg.SelectedPath }";
            let output = Command::new("powershell")
                .args(["-NoProfile", "-Command", script])
                .output()
                .ok()?;

            if !output.status.success() {
                return None;
            }

            let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if selected.is_empty() {
                None
            } else {
                Some(PathBuf::from(selected))
            }
        }

        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let zenity = Command::new("zenity")
                .args(["--file-selection", "--directory"])
                .output();

            if let Ok(output) = zenity {
                if output.status.success() {
                    let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !selected.is_empty() {
                        return Some(PathBuf::from(selected));
                    }
                }
            }

            let kdialog = Command::new("kdialog")
                .args(["--getexistingdirectory"])
                .output();

            if let Ok(output) = kdialog {
                if output.status.success() {
                    let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !selected.is_empty() {
                        return Some(PathBuf::from(selected));
                    }
                }
            }

            None
        }
    }

    fn maybe_zoom_into_folder(&mut self, path: &Path, viewport_rect: egui::Rect) -> bool {
        if !self.should_zoom_into_folder(path, viewport_rect) {
            return false;
        }

        // Preserve split/expand state so stepping back keeps previous view status.
        self.expansion_state.expand(path);
        self.view_root_path = Some(path.to_path_buf());
        true
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

    fn open_path(path: &std::path::Path) {
        #[cfg(target_os = "macos")]
        {
            let _ = Command::new("open").arg(path).spawn();
        }

        #[cfg(target_os = "windows")]
        {
            let _ = Command::new("cmd")
                .args(["/C", "start", ""])
                .arg(path)
                .spawn();
        }

        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let _ = Command::new("xdg-open").arg(path).spawn();
        }
    }

    fn open_in_file_manager(path: &std::path::Path, is_dir: bool) {
        #[cfg(target_os = "macos")]
        {
            if is_dir {
                let _ = Command::new("open").arg(path).spawn();
            } else {
                let _ = Command::new("open").arg("-R").arg(path).spawn();
            }
        }

        #[cfg(target_os = "windows")]
        {
            if is_dir {
                let _ = Command::new("explorer").arg(path).spawn();
            } else {
                let select_arg = format!("/select,{}", path.display());
                let _ = Command::new("explorer").arg(select_arg).spawn();
            }
        }

        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let target = if is_dir {
                path.to_path_buf()
            } else {
                path.parent().unwrap_or(path).to_path_buf()
            };
            let _ = Command::new("xdg-open").arg(target).spawn();
        }
    }

    fn open_in_file_manager_label() -> &'static str {
        #[cfg(target_os = "macos")]
        {
            "Open Folder in Finder"
        }

        #[cfg(target_os = "windows")]
        {
            "Open Folder in Explorer"
        }

        #[cfg(all(unix, not(target_os = "macos")))]
        {
            "Open Folder in File Manager"
        }
    }

    fn draw_centered_two_line_label(
        painter: &egui::Painter,
        rect: egui::Rect,
        name_text: &str,
        name_size: f32,
        size_text: &str,
        size_size: f32,
        name_color: egui::Color32,
        size_color: egui::Color32,
    ) -> bool {
        let available = rect.shrink2(egui::vec2(4.0, 4.0));
        if available.width() <= 0.0 || available.height() <= 0.0 {
            return false;
        }

        let name_font = egui::FontId::proportional(name_size);
        let size_font = egui::FontId::proportional(size_size);
        let size_galley = painter.layout_no_wrap(size_text.to_owned(), size_font.clone(), size_color);

        if size_galley.size().x > available.width() || size_galley.size().y >= available.height() {
            return false;
        }

        let line_probe = painter.layout_no_wrap("Ag".to_owned(), name_font.clone(), name_color);
        let name_line_height = line_probe.size().y.max(1.0);
        let spacing = 4.0;

        let name_height_budget = available.height() - size_galley.size().y - spacing;
        if name_height_budget < name_line_height * 0.9 {
            return false;
        }

        let max_rows_by_height = (name_height_budget / name_line_height).floor() as usize;
        let max_name_rows = max_rows_by_height.clamp(1, 3);

        let mut name_job = egui::text::LayoutJob::default();
        name_job.wrap.max_width = available.width();
        name_job.wrap.max_rows = max_name_rows;
        name_job.wrap.break_anywhere = true;
        name_job.append(
            name_text,
            0.0,
            egui::text::TextFormat::simple(name_font.clone(), name_color),
        );

        let name_galley = painter.layout_job(name_job);

        let max_width = name_galley.size().x.max(size_galley.size().x);
        let total_height = name_galley.size().y + size_galley.size().y + spacing;

        if max_width > available.width() || total_height > available.height() {
            return false;
        }

        if name_galley.rows.is_empty() {
            return false;
        }

        let top_y = available.center().y - total_height * 0.5;
        let name_pos = egui::pos2(available.center().x - name_galley.size().x * 0.5, top_y);
        let size_pos = egui::pos2(
            available.center().x - size_galley.size().x * 0.5,
            top_y + name_galley.size().y + spacing,
        );

        let clipped = painter.with_clip_rect(rect);
        clipped.galley(name_pos, name_galley, name_color);
        clipped.galley(size_pos, size_galley, size_color);

        true
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

    fn draw_scan_progress_circle(ui: &mut egui::Ui, progress: Option<&ScanProgress>) {
        let desired = egui::vec2(32.0, 32.0);
        let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::hover());
        let painter = ui.painter_at(rect);

        let center = rect.center();
        let radius = rect.width().min(rect.height()) * 0.5 - 2.0;

        painter.circle_stroke(
            center,
            radius,
            egui::Stroke::new(2.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 45)),
        );

        let (start, end, label) = if let Some(scan_progress) = progress {
            if let Some(frac) = scan_progress.fraction() {
                let clamped = frac.clamp(0.0, 1.0);
                let end = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * clamped;
                (
                    -std::f32::consts::FRAC_PI_2,
                    end,
                    format!("{}%", (clamped * 100.0).round() as u8),
                )
            } else {
                let time = ui.input(|i| i.time) as f32;
                let start = -std::f32::consts::FRAC_PI_2 + time * 2.4;
                let end = start + std::f32::consts::PI * 1.25;
                (start, end, "…".to_string())
            }
        } else {
            (
                -std::f32::consts::FRAC_PI_2,
                -std::f32::consts::FRAC_PI_2,
                "0%".to_string(),
            )
        };

        let sweep = (end - start).abs();
        if sweep > 0.0001 {
            let segments = ((sweep / std::f32::consts::TAU) * 64.0).ceil().max(8.0) as usize;
            let mut points = Vec::with_capacity(segments + 1);
            for i in 0..=segments {
                let t = i as f32 / segments as f32;
                let angle = start + (end - start) * t;
                points.push(egui::pos2(
                    center.x + angle.cos() * radius,
                    center.y + angle.sin() * radius,
                ));
            }

            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(3.0, egui::Color32::from_rgb(45, 212, 191)),
            ));
        }

        painter.text(
            center,
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(10.0),
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 210),
        );

        if let Some(scan_progress) = progress {
            let tooltip = if let Some(total) = scan_progress.total_entries {
                format!(
                    "Scanning {} / {} entries",
                    scan_progress.processed_entries, total
                )
            } else {
                format!("Discovering entries: {}", scan_progress.discovered_entries)
            };
            response.on_hover_text(tooltip);
        }
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
                header_response.context_menu(|ui| {
                    if ui.button("Open File or Folder").clicked() {
                        action = Some(ClickAction::OpenPath(node.path.clone()));
                        ui.close_menu();
                    }
                    if ui.button(Self::open_in_file_manager_label()).clicked() {
                        action = Some(ClickAction::OpenInFileManager {
                            path: node.path.clone(),
                            is_dir: true,
                        });
                        ui.close_menu();
                    }
                });
                if header_response.hovered() {
                    *hovered_path = Some(node.path.clone());
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    header_response.clone().on_hover_text(format!(
                        "{} ({})",
                        node.name,
                        Self::format_size(node.size)
                    ));
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
                if !node.is_aggregate {
                    response.context_menu(|ui| {
                        if ui.button("Open File or Folder").clicked() {
                            action = Some(ClickAction::OpenPath(node.path.clone()));
                            ui.close_menu();
                        }
                        if ui.button(Self::open_in_file_manager_label()).clicked() {
                            action = Some(ClickAction::OpenInFileManager {
                                path: node.path.clone(),
                                is_dir: node.is_dir,
                            });
                            ui.close_menu();
                        }
                    });
                }
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
                    let size_text = Self::format_size(node.size);
                    if area > min_label_area {
                        let _ = Self::draw_centered_two_line_label(
                            painter,
                            egui_rect,
                            &node.name,
                            11.0,
                            &size_text,
                            10.0,
                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 140),
                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 100),
                        );
                    }
                    if is_hovered {
                        response.on_hover_text(format!(
                            "{} ({}, {} items)",
                            node.name,
                            size_text,
                            node.aggregate_count
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
                        let size_text = Self::format_size(node.size);
                        let _ = Self::draw_centered_two_line_label(
                            painter,
                            egui_rect,
                            &name_text,
                            name_size,
                            &size_text,
                            size_size,
                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 255),
                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 153),
                        );
                    }
                    if is_hovered {
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
                let size_text = Self::format_size(item.size);
                let _ = Self::draw_centered_two_line_label(
                    painter,
                    egui_rect,
                    &name_text,
                    name_size,
                    &size_text,
                    size_size,
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, label_alpha),
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
        let layout_transition_active = self.update_layout_transition(dt);

        // If animation just finished, build the render tree
        let animation_just_finished =
            !still_animating && !self.animator.is_animating && self.has_data && self.render_nodes.is_empty();

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("SpaceInsight");
                ui.separator();

                ui.label("Path:");
                ui.text_edit_singleline(&mut self.scan_path);

                if ui.button("Browse...").clicked() {
                    if let Some(path) = Self::pick_folder_path() {
                        self.scan_path = path.display().to_string();
                    }
                }

                if ui.button("Scan").clicked() {
                    self.start_scan();
                }

                if self.is_scanning {
                    Self::draw_scan_progress_circle(ui, self.scan_progress.as_ref());
                    if let Some(progress) = &self.scan_progress {
                        if let Some(fraction) = progress.fraction() {
                            ui.label(format!(
                                "Scanning... {} / {} ({:.0}%)",
                                progress.processed_entries,
                                progress.total_entries.unwrap_or_default(),
                                fraction * 100.0,
                            ));
                        } else {
                            ui.label(format!(
                                "Scanning... discovering {} entries",
                                progress.discovered_entries
                            ));
                        }
                        ui.label(format!(
                            "Files: {}  Dirs: {}  Size: {}",
                            progress.total_files,
                            progress.total_dirs,
                            Self::format_size(progress.total_size)
                        ));
                    } else {
                        ui.label("Scanning...");
                    }
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
                    if self.view_root_path.is_some() && ui.button("Up Level").clicked() {
                        self.step_out_view_root();
                        if let Some(rect) = self.last_container_rect {
                            self.rebuild_render_tree(rect);
                        }
                    }
                    ui.label(format!("Root: {}", self.current_root_label()));
                    if let Some(size) = self.current_root_size() {
                        ui.separator();
                        ui.label(format!("Folder Size: {}", Self::format_size(size)));
                    }
                    if let Some(ref hovered) = self.hovered_path {
                        ui.separator();
                        ui.label(format!("{}", hovered.display()));
                    }
                });
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let available_rect = ui.available_rect_before_wrap();

            self.consume_file_events(available_rect);

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
            } else if self.is_scanning {
                self.render_live_scan_preview(ui, &painter, available_rect);
            } else {
                let display_nodes = self.current_display_nodes();

                if !display_nodes.is_empty() {
                    // Normal recursive rendering
                    let total_size = if let Some(tree) = &self.file_tree {
                        tree.total_size()
                    } else {
                        0
                    };
                    let min_label_area = self.animator.tier.min_label_area();

                    let action = Self::render_nodes_recursive(
                        &display_nodes,
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
                                if self.maybe_zoom_into_folder(&path, available_rect) {
                                    self.selected_path = None;
                                } else if !self.expansion_state.is_expanded(&path) {
                                    self.expansion_state.expand(&path);
                                }
                                self.rebuild_render_tree(available_rect);
                            }
                            ClickAction::Deepen(path) => {
                                if self.maybe_zoom_into_folder(&path, available_rect) {
                                    self.selected_path = None;
                                } else {
                                    // Double-click: expand this folder AND its child folders
                                    self.expansion_state.deepen(&path);
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
                            ClickAction::OpenPath(path) => {
                                Self::open_path(&path);
                            }
                            ClickAction::OpenInFileManager { path, is_dir } => {
                                Self::open_in_file_manager(&path, is_dir);
                            }
                        }
                    }
                }
            }

            self.hovered_path = new_hovered_path;

            if self.is_scanning
                || still_animating
                || self.animator.is_animating
                || layout_transition_active
            {
                ctx.request_repaint();
            }
        });
    }
}
