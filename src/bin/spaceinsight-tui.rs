use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect as UiRect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};
use ratatui::{Frame, Terminal};
use spaceinsight::crawler::{FileCrawler, ScanPhase, ScanProgress, ScanStats};
use spaceinsight::expand_state::ExpansionState;
use spaceinsight::render_tree::{build_render_tree, RenderNode};
use spaceinsight::tree::FileTree;
use spaceinsight::treemap::Rect;
use std::cmp::Ordering;
use std::io::{self, stdout};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const MAX_RENDER_DEPTH: usize = 5;

enum ScanEvent {
    Progress(ScanProgress),
    Completed(Result<ScanResult, String>),
}

struct ScanResult {
    tree: FileTree,
    stats: ScanStats,
}

#[derive(Clone)]
struct VisibleTile {
    path: PathBuf,
    name: String,
    size: u64,
    is_dir: bool,
    is_aggregate: bool,
    depth: u16,
    rect: Rect,
}

#[derive(Clone)]
struct HitTile {
    path: PathBuf,
    is_dir: bool,
    is_aggregate: bool,
    size: u64,
    depth: u16,
    x0: u16,
    y0: u16,
    x1: u16,
    y1: u16,
}

impl HitTile {
    fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
}

#[derive(Default, Clone, Copy)]
struct UiLayoutState {
    path_input_area: Option<UiRect>,
    treemap_inner_area: Option<UiRect>,
}

struct App {
    path_input: String,
    input_mode: bool,
    status: String,
    is_scanning: bool,
    scan_progress: Option<ScanProgress>,
    scan_rx: Option<Receiver<ScanEvent>>,
    last_scan_finished_at: Option<Instant>,

    file_tree: Option<FileTree>,
    view_root_path: Option<PathBuf>,
    expansion_state: ExpansionState,

    selected_path: Option<PathBuf>,
    selected_size: Option<u64>,
    selected_is_dir: bool,
    hit_tiles: Vec<HitTile>,
    ui_layout: UiLayoutState,

    should_quit: bool,
}

impl Default for App {
    fn default() -> Self {
        Self {
            path_input: String::from("."),
            input_mode: true,
            status: String::from("Type path and press Enter to scan"),
            is_scanning: false,
            scan_progress: None,
            scan_rx: None,
            last_scan_finished_at: None,
            file_tree: None,
            view_root_path: None,
            expansion_state: ExpansionState::default(),
            selected_path: None,
            selected_size: None,
            selected_is_dir: false,
            hit_tiles: Vec::new(),
            ui_layout: UiLayoutState::default(),
            should_quit: false,
        }
    }
}

impl App {
    fn start_scan(&mut self) {
        if self.is_scanning {
            return;
        }

        let path = if self.path_input.trim().is_empty() {
            ".".to_string()
        } else {
            self.path_input.trim().to_string()
        };

        if !Path::new(&path).exists() {
            self.status = format!("Path does not exist: {}", path);
            return;
        }

        self.path_input = path.clone();
        self.status = format!("Scanning {} ...", path);
        self.is_scanning = true;
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

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let (nodes, stats) = crawler.scan_with_progress(&path, Some(reporter));
                let mut tree = FileTree::new(&path);
                for node in nodes {
                    tree.upsert_node(node.path, node.size, node.is_dir);
                }
                tree.calculate_sizes();
                ScanResult { tree, stats }
            }));

            let event = match result {
                Ok(scan_result) => ScanEvent::Completed(Ok(scan_result)),
                Err(_) => ScanEvent::Completed(Err("Scan thread panicked".to_string())),
            };

            let _ = tx.send(event);
        });
    }

    fn poll_scan_updates(&mut self) {
        let mut done: Option<Result<ScanResult, String>> = None;
        let mut disconnected = false;

        if let Some(rx) = self.scan_rx.as_ref() {
            loop {
                match rx.try_recv() {
                    Ok(ScanEvent::Progress(progress)) => {
                        self.scan_progress = Some(progress);
                    }
                    Ok(ScanEvent::Completed(result)) => {
                        done = Some(result);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }

        if disconnected {
            self.scan_rx = None;
            self.is_scanning = false;
            if done.is_none() {
                self.status = "Scan channel disconnected".to_string();
            }
        }

        if let Some(result) = done {
            self.scan_rx = None;
            self.is_scanning = false;
            self.scan_progress = None;

            match result {
                Ok(scan_result) => {
                    self.file_tree = Some(scan_result.tree);
                    self.view_root_path = None;
                    self.expansion_state.collapse_all();
                    self.selected_path = None;
                    self.selected_size = None;
                    self.selected_is_dir = false;
                    self.last_scan_finished_at = Some(Instant::now());
                    self.status = format!(
                        "Scan complete: {} files, {} dirs, {} total",
                        scan_result.stats.total_files,
                        scan_result.stats.total_dirs,
                        format_size(scan_result.stats.total_size),
                    );
                }
                Err(err) => {
                    self.status = format!("Scan failed: {}", err);
                }
            }
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
            self.should_quit = true;
            return;
        }

        if self.input_mode {
            match key.code {
                KeyCode::Enter => {
                    self.input_mode = false;
                    self.start_scan();
                }
                KeyCode::Esc => {
                    self.input_mode = false;
                }
                KeyCode::Backspace => {
                    self.path_input.pop();
                }
                KeyCode::Char(ch) => {
                    self.path_input.push(ch);
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('/') => self.input_mode = true,
            KeyCode::Char('s') => self.start_scan(),
            KeyCode::Char('r') => self.start_scan(),
            KeyCode::Char('e') => self.expand_selected(),
            KeyCode::Char('d') => self.deepen_selected(),
            KeyCode::Char('c') => self.collapse_selected(),
            KeyCode::Char('z') => self.zoom_into_selected(),
            KeyCode::Char('u') | KeyCode::Backspace => self.zoom_out_one_level(),
            KeyCode::Esc => {
                self.selected_path = None;
                self.selected_size = None;
                self.selected_is_dir = false;
            }
            _ => {}
        }
    }

    fn on_mouse(&mut self, event: MouseEvent) {
        if let MouseEventKind::Down(MouseButton::Left) = event.kind {
            if let Some(path_input_area) = self.ui_layout.path_input_area {
                if point_in_rect(path_input_area, event.column, event.row) {
                    self.input_mode = true;
                    return;
                }
            }

            if let Some(tile) = self.tile_at(event.column, event.row).cloned() {
                if tile.is_aggregate {
                    self.status = format!("{} selected", format_size(tile.size));
                    return;
                }
                self.select_tile(&tile);
                if tile.is_dir {
                    if self.expansion_state.is_expanded(&tile.path) {
                        self.expansion_state.collapse_recursive(&tile.path);
                    } else {
                        self.expansion_state.expand(&tile.path);
                    }
                }
            }
            return;
        }

        if let MouseEventKind::Down(MouseButton::Right) = event.kind {
            if let Some(tile) = self.tile_at(event.column, event.row).cloned() {
                if tile.is_dir {
                    self.select_tile(&tile);
                    self.view_root_path = Some(tile.path);
                    self.expansion_state.collapse_all();
                }
            }
            return;
        }

        if let MouseEventKind::Down(MouseButton::Middle) = event.kind {
            self.zoom_out_one_level();
        }
    }

    fn expand_selected(&mut self) {
        if !self.selected_is_dir {
            return;
        }
        if let Some(path) = self.selected_path.as_ref() {
            self.expansion_state.expand(path);
        }
    }

    fn deepen_selected(&mut self) {
        if !self.selected_is_dir {
            return;
        }
        if let Some(path) = self.selected_path.as_ref() {
            self.expansion_state.deepen(path);
        }
    }

    fn collapse_selected(&mut self) {
        if !self.selected_is_dir {
            return;
        }
        if let Some(path) = self.selected_path.as_ref() {
            self.expansion_state.collapse_recursive(path);
        }
    }

    fn zoom_into_selected(&mut self) {
        if !self.selected_is_dir {
            return;
        }
        if let Some(path) = self.selected_path.as_ref() {
            self.view_root_path = Some(path.clone());
            self.expansion_state.collapse_all();
        }
    }

    fn zoom_out_one_level(&mut self) {
        let Some(tree) = self.file_tree.as_ref() else {
            return;
        };

        let Some(current) = self.current_view_root_path() else {
            return;
        };

        let Some(root_path) = tree.root_path() else {
            self.view_root_path = None;
            return;
        };

        if current == root_path {
            self.view_root_path = None;
            return;
        }

        let parent = current.parent().map(Path::to_path_buf);
        match parent {
            Some(parent_path) if parent_path.starts_with(root_path) => {
                if parent_path == root_path {
                    self.view_root_path = None;
                } else {
                    self.view_root_path = Some(parent_path);
                }
            }
            _ => {
                self.view_root_path = None;
            }
        }
        self.expansion_state.collapse_all();
    }

    fn current_view_root_path(&self) -> Option<&Path> {
        let tree = self.file_tree.as_ref()?;
        if let Some(path) = self.view_root_path.as_deref() {
            Some(path)
        } else {
            tree.root_path()
        }
    }

    fn current_view_root_node(&self) -> Option<indextree::NodeId> {
        let tree = self.file_tree.as_ref()?;
        if let Some(path) = self.view_root_path.as_ref() {
            tree.get_node(path).or_else(|| Some(tree.get_root()))
        } else {
            Some(tree.get_root())
        }
    }

    fn build_visible_tiles(&mut self, area: UiRect) -> Vec<VisibleTile> {
        let Some(tree) = self.file_tree.as_ref() else {
            self.hit_tiles.clear();
            return Vec::new();
        };

        let Some(view_root_id) = self.current_view_root_node() else {
            self.hit_tiles.clear();
            return Vec::new();
        };

        if area.width <= 2 || area.height <= 2 {
            self.hit_tiles.clear();
            return Vec::new();
        }

        let layout_container = Rect::new(0.0, 0.0, area.width as f32, area.height as f32);
        let render_nodes = build_render_tree(
            tree,
            view_root_id,
            layout_container,
            &self.expansion_state,
            MAX_RENDER_DEPTH,
        );

        let mut tiles = Vec::new();
        flatten_render_nodes(&render_nodes, 0, &mut tiles);
        self.hit_tiles = tiles
            .iter()
            .filter_map(|tile| {
                tile_bounds_in_area(tile, area).map(|(x0, y0, x1, y1)| HitTile {
                    path: tile.path.clone(),
                    is_dir: tile.is_dir,
                    is_aggregate: tile.is_aggregate,
                    size: tile.size,
                    depth: tile.depth,
                    x0,
                    y0,
                    x1,
                    y1,
                })
            })
            .collect();

        tiles
    }

    fn tile_at(&self, x: u16, y: u16) -> Option<&HitTile> {
        self.hit_tiles
            .iter()
            .filter(|tile| tile.contains(x, y))
            .max_by(|a, b| {
                let depth = a.depth.cmp(&b.depth);
                if depth != Ordering::Equal {
                    return depth;
                }
                let area_a = (a.x1.saturating_sub(a.x0) as u32) * (a.y1.saturating_sub(a.y0) as u32);
                let area_b = (b.x1.saturating_sub(b.x0) as u32) * (b.y1.saturating_sub(b.y0) as u32);
                area_b.cmp(&area_a)
            })
    }

    fn select_tile(&mut self, tile: &HitTile) {
        self.selected_path = Some(tile.path.clone());
        self.selected_size = Some(tile.size);
        self.selected_is_dir = tile.is_dir;
        self.status = format!(
            "Selected {} ({})",
            tile.path.display(),
            format_size(tile.size)
        );
    }
}

fn flatten_render_nodes(nodes: &[RenderNode], depth: u16, out: &mut Vec<VisibleTile>) {
    for node in nodes {
        out.push(VisibleTile {
            path: node.path.clone(),
            name: node.name.clone(),
            size: node.size,
            is_dir: node.is_dir,
            is_aggregate: node.is_aggregate,
            depth,
            rect: node.outer_rect,
        });

        if !node.children.is_empty() {
            flatten_render_nodes(&node.children, depth.saturating_add(1), out);
        }
    }
}

fn tile_bounds_in_area(tile: &VisibleTile, area: UiRect) -> Option<(u16, u16, u16, u16)> {
    if area.width == 0 || area.height == 0 {
        return None;
    }

    let max_x = area.x.saturating_add(area.width.saturating_sub(1));
    let max_y = area.y.saturating_add(area.height.saturating_sub(1));

    let mut x0 = area.x.saturating_add(tile.rect.x.floor().max(0.0) as u16);
    let mut y0 = area.y.saturating_add(tile.rect.y.floor().max(0.0) as u16);
    let mut x1 = area
        .x
        .saturating_add(((tile.rect.x + tile.rect.width).ceil().max(1.0) as u16).saturating_sub(1));
    let mut y1 = area
        .y
        .saturating_add(((tile.rect.y + tile.rect.height).ceil().max(1.0) as u16).saturating_sub(1));

    x0 = x0.clamp(area.x, max_x);
    y0 = y0.clamp(area.y, max_y);
    x1 = x1.clamp(area.x, max_x);
    y1 = y1.clamp(area.y, max_y);

    if x1 < x0 || y1 < y0 {
        return None;
    }
    Some((x0, y0, x1, y1))
}

fn point_in_rect(rect: UiRect, x: u16, y: u16) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}

fn format_size(size: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = size as f64;
    let mut unit_index = 0;
    while value >= 1024.0 && unit_index < UNITS.len() - 1 {
        value /= 1024.0;
        unit_index += 1;
    }
    format!("{:.1} {}", value, UNITS[unit_index])
}

fn progress_status(progress: &ScanProgress) -> String {
    match progress.phase {
        ScanPhase::Discovering => format!(
            "Discovering entries... {} found",
            progress.discovered_entries
        ),
        ScanPhase::Processing => {
            if let Some(frac) = progress.fraction() {
                format!(
                    "Processing {:.0}% | files: {} dirs: {} | {}",
                    frac * 100.0,
                    progress.total_files,
                    progress.total_dirs,
                    format_size(progress.total_size),
                )
            } else {
                format!(
                    "Processing... files: {} dirs: {} | {}",
                    progress.total_files,
                    progress.total_dirs,
                    format_size(progress.total_size),
                )
            }
        }
    }
}

fn tile_color(tile: &VisibleTile, max_size: u64) -> Color {
    if tile.is_aggregate {
        return Color::Rgb(80, 80, 84);
    }

    let ratio = if max_size == 0 {
        0.0
    } else {
        (tile.size as f32 / max_size as f32).clamp(0.0, 1.0)
    };

    if tile.is_dir {
        let r = (35.0 + ratio * 65.0) as u8;
        let g = (95.0 + ratio * 95.0) as u8;
        let b = (145.0 + ratio * 85.0) as u8;
        Color::Rgb(r, g, b)
    } else {
        let r = (55.0 + ratio * 120.0) as u8;
        let g = (85.0 + ratio * 95.0) as u8;
        let b = (95.0 + ratio * 70.0) as u8;
        Color::Rgb(r, g, b)
    }
}

struct TreemapWidget<'a> {
    tiles: &'a [VisibleTile],
    selected_path: Option<&'a PathBuf>,
}

impl<'a> TreemapWidget<'a> {
    fn new(tiles: &'a [VisibleTile], selected_path: Option<&'a PathBuf>) -> Self {
        Self { tiles, selected_path }
    }
}

impl Widget for TreemapWidget<'_> {
    fn render(self, area: UiRect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        for y in area.y..area.y.saturating_add(area.height) {
            for x in area.x..area.x.saturating_add(area.width) {
                buf[(x, y)]
                    .set_char(' ')
                    .set_style(Style::default().bg(Color::Rgb(18, 18, 20)));
            }
        }

        let max_size = self.tiles.iter().map(|tile| tile.size).max().unwrap_or(0);
        let mut draw_order = self.tiles.to_vec();
        draw_order.sort_by_key(|tile| tile.depth);

        for tile in &draw_order {
            let Some((x0, y0, x1, y1)) = tile_bounds_in_area(tile, area) else {
                continue;
            };

            let is_selected = self
                .selected_path
                .map(|selected| selected == &tile.path)
                .unwrap_or(false);

            let bg = tile_color(tile, max_size);
            let border_color = if is_selected {
                Color::Rgb(246, 211, 101)
            } else {
                Color::Rgb(224, 224, 224)
            };

            for y in y0..=y1 {
                for x in x0..=x1 {
                    buf[(x, y)]
                        .set_char(' ')
                        .set_style(Style::default().bg(bg).fg(Color::White));
                }
            }

            if x1 > x0 {
                for x in x0..=x1 {
                    buf[(x, y0)]
                        .set_char('─')
                        .set_style(Style::default().fg(border_color).bg(bg));
                    buf[(x, y1)]
                        .set_char('─')
                        .set_style(Style::default().fg(border_color).bg(bg));
                }
            }

            if y1 > y0 {
                for y in y0..=y1 {
                    buf[(x0, y)]
                        .set_char('│')
                        .set_style(Style::default().fg(border_color).bg(bg));
                    buf[(x1, y)]
                        .set_char('│')
                        .set_style(Style::default().fg(border_color).bg(bg));
                }
            }

            buf[(x0, y0)]
                .set_char('┌')
                .set_style(Style::default().fg(border_color).bg(bg));
            buf[(x1, y0)]
                .set_char('┐')
                .set_style(Style::default().fg(border_color).bg(bg));
            buf[(x0, y1)]
                .set_char('└')
                .set_style(Style::default().fg(border_color).bg(bg));
            buf[(x1, y1)]
                .set_char('┘')
                .set_style(Style::default().fg(border_color).bg(bg));

            let label_width = x1.saturating_sub(x0).saturating_sub(1) as usize;
            if label_width >= 4 {
                let label = if tile.is_aggregate {
                    tile.name.clone()
                } else {
                    format!("{} {}", tile.name, format_size(tile.size))
                };
                let mut label = label;
                if label.len() > label_width {
                    label.truncate(label_width.saturating_sub(1));
                    label.push('…');
                }

                for (i, ch) in label.chars().enumerate() {
                    let x = x0.saturating_add(1).saturating_add(i as u16);
                    if x > x1.saturating_sub(1) {
                        break;
                    }
                    buf[(x, y0)]
                        .set_char(ch)
                        .set_style(Style::default().fg(Color::White).bg(bg).add_modifier(Modifier::BOLD));
                }
            }
        }
    }
}

fn draw_ui(frame: &mut Frame, app: &mut App) {
    let root = frame.area();
    let split = Layout::horizontal([Constraint::Length(42), Constraint::Min(30)]).split(root);
    let left = split[0];
    let right = split[1];

    let left_block = Block::default()
        .title(" SpaceInsight TUI ")
        .borders(Borders::ALL);
    let left_inner = left_block.inner(left);
    frame.render_widget(left_block, left);

    let left_rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(6),
        Constraint::Min(8),
        Constraint::Length(4),
    ])
    .split(left_inner);

    let input_title = if app.input_mode {
        " Path (typing) "
    } else {
        " Path "
    };
    let path_block = Block::default().title(input_title).borders(Borders::ALL);
    let path_inner = path_block.inner(left_rows[0]);
    frame.render_widget(path_block, left_rows[0]);
    let path_style = if app.input_mode {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::White)
    };
    frame.render_widget(
        Paragraph::new(app.path_input.as_str()).style(path_style),
        path_inner,
    );

    let progress_text = if app.is_scanning {
        app.scan_progress
            .as_ref()
            .map(progress_status)
            .unwrap_or_else(|| "Scanning...".to_string())
    } else {
        app.status.clone()
    };
    let progress = Paragraph::new(progress_text)
        .block(Block::default().title(" Status ").borders(Borders::ALL));
    frame.render_widget(progress, left_rows[1]);

    let selected_lines = {
        let mut lines = Vec::new();
        let view_root = app
            .current_view_root_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(none)".to_string());
        lines.push(Line::from(vec![
            Span::styled("View: ", Style::default().fg(Color::Gray)),
            Span::raw(view_root),
        ]));

        if let Some(path) = app.selected_path.as_ref() {
            lines.push(Line::from(vec![
                Span::styled("Selected: ", Style::default().fg(Color::Gray)),
                Span::raw(path.display().to_string()),
            ]));
            lines.push(Line::from(vec![
                Span::styled("Type: ", Style::default().fg(Color::Gray)),
                Span::raw(if app.selected_is_dir { "directory" } else { "file" }),
                Span::raw("  "),
                Span::styled("Size: ", Style::default().fg(Color::Gray)),
                Span::raw(
                    app.selected_size
                        .map(format_size)
                        .unwrap_or_else(|| "n/a".to_string()),
                ),
            ]));
        } else {
            lines.push(Line::from("Selected: (none)"));
        }

        if let Some(instant) = app.last_scan_finished_at {
            lines.push(Line::from(format!(
                "Last scan: {}s ago",
                instant.elapsed().as_secs()
            )));
        }
        lines
    };

    frame.render_widget(
        Paragraph::new(selected_lines)
            .block(Block::default().title(" Selection ").borders(Borders::ALL)),
        left_rows[2],
    );

    let help_lines = vec![
        Line::from("Enter: scan path    /: edit path"),
        Line::from("Left click: select + expand"),
        Line::from("Right click/z: zoom in   u: up"),
        Line::from("e/d/c: expand/deepen/collapse   q: quit"),
    ];
    frame.render_widget(
        Paragraph::new(help_lines).block(Block::default().title(" Controls ").borders(Borders::ALL)),
        left_rows[3],
    );

    app.ui_layout.path_input_area = Some(path_inner);

    let treemap_block = Block::default()
        .title(" Treemap (left click: expand/collapse, right click: zoom) ")
        .borders(Borders::ALL);
    let treemap_inner = treemap_block.inner(right);
    frame.render_widget(treemap_block, right);
    app.ui_layout.treemap_inner_area = Some(treemap_inner);

    let tiles = app.build_visible_tiles(treemap_inner);
    if tiles.is_empty() {
        frame.render_widget(
            Paragraph::new("No treemap yet. Enter a path and scan.")
                .style(Style::default().fg(Color::Gray)),
            treemap_inner,
        );
    } else {
        frame.render_widget(
            TreemapWidget::new(&tiles, app.selected_path.as_ref()),
            treemap_inner,
        );
    }
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> io::Result<()> {
    let mut app = App::default();

    loop {
        app.poll_scan_updates();

        terminal.draw(|frame| {
            draw_ui(frame, &mut app);
        })?;

        if app.should_quit {
            break;
        }

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => app.on_key(key),
                Event::Mouse(mouse) => app.on_mouse(mouse),
                Event::Resize(_, _) => {}
                Event::FocusGained | Event::FocusLost | Event::Paste(_) => {}
            }
        }
    }

    Ok(())
}

fn main() -> io::Result<()> {
    enable_raw_mode()?;
    crossterm::execute!(stdout(), EnterAlternateScreen, EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let app_result = run_app(&mut terminal);

    disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    app_result
}
