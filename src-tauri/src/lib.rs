use serde::{Deserialize, Serialize};
use spaceinsight::crawler::{FileCrawler, ScanPhase, ScanProgress};
use spaceinsight::expand_state::ExpansionState;
use spaceinsight::render_tree::build_render_tree;
use spaceinsight::tree::FileTree;
use spaceinsight::treemap::{Rect, SquarifiedTreemap, TreemapItem};
use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Default)]
struct AppData {
    scanning: bool,
    tree: Option<FileTree>,
    scanned_root: Option<PathBuf>,
    total_files: u64,
    total_dirs: u64,
}

struct AppState(Mutex<AppData>);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressPayload {
    phase: &'static str,
    processed_entries: u64,
    total_files: u64,
    total_dirs: u64,
    total_size: u64,
    preview: Vec<PreviewItem>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewItem {
    path: String,
    name: String,
    size: u64,
    is_dir: bool,
}

impl ProgressPayload {
    fn from_progress(progress: ScanProgress, root: &Path) -> Self {
        Self {
            phase: match progress.phase {
                ScanPhase::Discovering => "discovering",
                ScanPhase::Processing => "processing",
            },
            processed_entries: progress.processed_entries,
            total_files: progress.total_files,
            total_dirs: progress.total_dirs,
            total_size: progress.total_size,
            preview: progress
                .top_level_preview
                .into_iter()
                .map(|item| PreviewItem {
                    path: root.join(&item.name).to_string_lossy().into_owned(),
                    name: item.name,
                    size: item.size,
                    is_dir: item.is_dir,
                })
                .collect(),
        }
    }
}

fn path_hash(path: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanCompletePayload {
    root: String,
    total_size: u64,
    total_files: u64,
    total_dirs: u64,
    duration_ms: u128,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LayoutNode {
    id: u64,
    path: String,
    name: String,
    size: u64,
    is_dir: bool,
    is_aggregate: bool,
    aggregate_count: usize,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LayoutResponse {
    root: String,
    scanned_root: String,
    parent: Option<String>,
    total_size: u64,
    total_files: u64,
    total_dirs: u64,
    nodes: Vec<LayoutNode>,
}

#[tauri::command]
fn default_path() -> String {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .to_string_lossy()
        .into_owned()
}

#[tauri::command]
fn start_scan(path: String, app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let requested = PathBuf::from(path.trim());
    if !requested.is_dir() {
        return Err("Choose a directory that exists and can be read".to_string());
    }

    let root = requested.canonicalize().unwrap_or(requested);
    {
        let mut data = state.0.lock().map_err(|_| "Scanner state is unavailable")?;
        if data.scanning {
            return Err("A scan is already running".to_string());
        }
        data.scanning = true;
    }

    let root_label = root.to_string_lossy().into_owned();
    std::thread::spawn(move || {
        let reporter_app = app.clone();
        let reporter_root = root.clone();
        let reporter = Arc::new(move |progress: ScanProgress| {
            let _ = reporter_app.emit(
                "scan-progress",
                ProgressPayload::from_progress(progress, &reporter_root),
            );
        });

        let mut crawler = FileCrawler::new();
        let (nodes, stats) = crawler.scan_with_progress(&root, Some(reporter));
        let mut tree = FileTree::new(&root);
        for node in nodes {
            tree.upsert_node(node.path, node.size, node.is_dir);
        }
        tree.calculate_sizes();
        let total_size = tree.total_size();

        if let Some(app_state) = app.try_state::<AppState>() {
            if let Ok(mut data) = app_state.0.lock() {
                data.scanning = false;
                data.tree = Some(tree);
                data.scanned_root = Some(root.clone());
                data.total_files = stats.total_files;
                data.total_dirs = stats.total_dirs;
            }
        }

        let _ = app.emit(
            "scan-complete",
            ScanCompletePayload {
                root: root_label,
                total_size,
                total_files: stats.total_files,
                total_dirs: stats.total_dirs,
                duration_ms: stats.duration_ms,
            },
        );
    });

    Ok(())
}

#[tauri::command]
fn get_preview_layout(preview: Vec<PreviewItem>, width: f32, height: f32) -> Vec<LayoutNode> {
    let items: Vec<TreemapItem> = preview
        .iter()
        .enumerate()
        .filter(|(_, item)| item.size > 0)
        .map(|(index, item)| TreemapItem {
            size: item.size,
            index,
        })
        .collect();
    let layout =
        SquarifiedTreemap::layout(&items, Rect::new(0.0, 0.0, width.max(1.0), height.max(1.0)));

    layout
        .into_iter()
        .filter_map(|item| {
            let preview_item = preview.get(item.index)?;
            Some(LayoutNode {
                id: path_hash(&preview_item.path),
                path: preview_item.path.clone(),
                name: preview_item.name.clone(),
                size: preview_item.size,
                is_dir: preview_item.is_dir,
                is_aggregate: false,
                aggregate_count: 0,
                x: item.rect.x,
                y: item.rect.y,
                width: item.rect.width,
                height: item.rect.height,
            })
        })
        .collect()
}

#[tauri::command]
fn get_layout(
    path: String,
    width: f32,
    height: f32,
    state: State<'_, AppState>,
) -> Result<LayoutResponse, String> {
    let data = state.0.lock().map_err(|_| "Scanner state is unavailable")?;
    let tree = data.tree.as_ref().ok_or("Scan a folder first")?;
    let scanned_root = data.scanned_root.as_ref().ok_or("Scan a folder first")?;
    let requested = PathBuf::from(path);
    if !requested.starts_with(scanned_root) {
        return Err("The requested folder is outside the current scan".to_string());
    }
    let node_id = tree
        .get_node(&requested)
        .ok_or_else(|| "This folder is no longer in the scan".to_string())?;
    let node = tree
        .get_arena()
        .get(node_id)
        .ok_or_else(|| "Unable to read this folder".to_string())?;
    if !node.get().is_dir {
        return Err("Only folders can be opened in the treemap".to_string());
    }

    let layout = build_render_tree(
        tree,
        node_id,
        Rect::new(0.0, 0.0, width.max(1.0), height.max(1.0)),
        &ExpansionState::default(),
        1,
        &HashSet::new(),
    );
    let nodes = layout
        .into_iter()
        .map(|item| LayoutNode {
            id: item.stable_id,
            path: item.path.to_string_lossy().into_owned(),
            name: item.name,
            size: item.size,
            is_dir: item.is_dir,
            is_aggregate: item.is_aggregate,
            aggregate_count: item.aggregate_count,
            x: item.outer_rect.x,
            y: item.outer_rect.y,
            width: item.outer_rect.width,
            height: item.outer_rect.height,
        })
        .collect();

    let parent = requested.parent().and_then(|candidate| {
        if candidate.starts_with(scanned_root) {
            Some(candidate.to_string_lossy().into_owned())
        } else {
            None
        }
    });

    Ok(LayoutResponse {
        root: requested.to_string_lossy().into_owned(),
        scanned_root: scanned_root.to_string_lossy().into_owned(),
        parent,
        total_size: node.get().cumulative_size,
        total_files: data.total_files,
        total_dirs: data.total_dirs,
        nodes,
    })
}

#[tauri::command]
fn reveal_path(path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    if !path.exists() {
        return Err("The requested path no longer exists".to_string());
    }

    #[cfg(target_os = "macos")]
    let result = if path.is_dir() {
        Command::new("open").arg(&path).spawn()
    } else {
        Command::new("open").arg("-R").arg(&path).spawn()
    };
    #[cfg(target_os = "windows")]
    let result = if path.is_dir() {
        Command::new("explorer").arg(&path).spawn()
    } else {
        Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn()
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = Command::new("xdg-open")
        .arg(if path.is_dir() {
            path.as_path()
        } else {
            path.parent().unwrap_or(&path)
        })
        .spawn();

    result
        .map(|_| ())
        .map_err(|error| format!("Unable to open the file manager: {error}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState(Mutex::new(AppData::default())))
        .invoke_handler(tauri::generate_handler![
            default_path,
            start_scan,
            get_preview_layout,
            get_layout,
            reveal_path
        ])
        .run(tauri::generate_context!())
        .expect("error while running SpaceInsight");
}
