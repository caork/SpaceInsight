#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod input;
mod scan;

use gpui::{prelude::*, *};
use spaceinsight::{
    crawler::ScanProgress,
    expand_state::ExpansionState,
    render_tree::{RenderNode, build_render_tree},
    tree::FileTree,
    treemap::{Rect, SquarifiedTreemap, TreemapItem},
};
use std::{cell::RefCell, collections::HashSet, path::PathBuf, rc::Rc, sync::Arc, time::Duration};

actions!(spaceinsight, [Scan, Up, ChooseFolder, Quit]);

struct SpaceInsight {
    input: Entity<input::TextInput>,
    tree: Option<Arc<FileTree>>,
    root: Option<PathBuf>,
    current: Option<PathBuf>,
    selected: Option<PathBuf>,
    progress: Option<ScanProgress>,
    scan_root: Option<PathBuf>,
    scanning: bool,
    files: u64,
    dirs: u64,
    status: String,
    focus: FocusHandle,
    _scan_task: Option<Task<()>>,
}

fn bytes(value: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB", "PB", "EB"];
    let mut amount = value as f64;
    let mut unit = 0;
    while amount >= 1000. && unit < units.len() - 1 {
        amount /= 1000.;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} B")
    } else {
        format!("{amount:.1} {}", units[unit])
    }
}

fn button(id: &'static str, label: impl Into<SharedString>, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .px_3()
        .py_2()
        .rounded_md()
        .text_sm()
        .bg(rgb(if enabled { 0xe7edf2 } else { 0xf1f3f5 }))
        .text_color(rgb(if enabled { 0x263849 } else { 0x9aa5af }))
        .when(enabled, |el| {
            el.cursor_pointer().hover(|s| s.bg(rgb(0xd9e7ed)))
        })
        .child(label.into())
}

impl SpaceInsight {
    fn start_scan(&mut self, _: &Scan, _: &mut Window, cx: &mut Context<Self>) {
        if self.scanning {
            return;
        }
        let requested = PathBuf::from(self.input.read(cx).content.as_str().trim());
        let root = match scan::validate_root(&requested) {
            Ok(root) => root,
            Err(error) => {
                self.status = error;
                cx.notify();
                return;
            }
        };
        self.scanning = true;
        self.progress = None;
        self.scan_root = Some(root.clone());
        self.selected = None;
        self.status = "Reading directory entries…".into();
        let (progress, result) = scan::start(root);
        self._scan_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let latest = progress.lock().ok().and_then(|mut p| p.take());
                let complete = result.try_recv();
                let done = !matches!(complete, Err(std::sync::mpsc::TryRecvError::Empty));
                if this
                    .update(cx, |view, cx| {
                        if let Some(progress) = latest {
                            view.status = format!(
                                "Scanning · {} files · {} folders · {} · {} unreadable",
                                progress.total_files,
                                progress.total_dirs,
                                bytes(progress.total_size),
                                progress.unreadable_entries
                            );
                            view.progress = Some(progress);
                        }
                        match complete {
                            Ok(Ok(scan)) => {
                                view.files = scan.stats.total_files;
                                view.dirs = scan.stats.total_dirs;
                                view.root = Some(scan.root.clone());
                                view.current = Some(scan.root);
                                view.tree = Some(Arc::new(scan.tree));
                                view.status = format!(
                                    "Scan complete · {:.2}s · {} unreadable entries skipped",
                                    scan.stats.duration_ms as f64 / 1000.,
                                    scan.stats.unreadable_entries
                                );
                            }
                            Ok(Err(error)) => view.status = error,
                            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                                view.status =
                                    "Scanner stopped unexpectedly. Try scanning again.".into()
                            }
                            _ => {}
                        }
                        if done {
                            view.scanning = false;
                            view.progress = None;
                            view.scan_root = None;
                        }
                        cx.notify();
                    })
                    .is_err()
                    || done
                {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn choose_folder(&mut self, _: &ChooseFolder, _: &mut Window, cx: &mut Context<Self>) {
        if self.scanning {
            return;
        }
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose folder".into()),
        });
        cx.spawn(async move |this, cx| match picker.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.first() {
                    let _ = this.update(cx, |view, cx| {
                        view.input.update(cx, |input, cx| {
                            input.set_content(path.to_string_lossy().into_owned(), cx)
                        });
                        view.status = "Folder selected. Press Scan to begin.".into();
                        cx.notify();
                    });
                }
            }
            Ok(Err(error)) => {
                let _ = this.update(cx, |view, cx| {
                    view.status = format!("Unable to choose folder: {error}");
                    cx.notify();
                });
            }
            _ => {}
        })
        .detach();
    }

    fn navigate(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.scanning {
            return;
        }
        if self
            .root
            .as_ref()
            .is_some_and(|root| path.starts_with(root))
            && self
                .tree
                .as_ref()
                .is_some_and(|tree| tree.get_node(&path).is_some())
        {
            self.current = Some(path);
            self.selected = None;
            cx.notify();
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(parent) = self
            .current
            .as_ref()
            .and_then(|p| p.parent())
            .map(PathBuf::from)
        {
            self.navigate(parent, cx);
        }
    }

    fn copy_path(&mut self, cx: &mut Context<Self>) {
        if self.scanning {
            return;
        }
        if let Some(path) = self.selected.as_ref().or(self.current.as_ref()) {
            cx.write_to_clipboard(ClipboardItem::new_string(
                path.to_string_lossy().into_owned(),
            ));
            self.status = "Path copied".into();
            cx.notify();
        }
    }

    fn reveal(&mut self, cx: &mut Context<Self>) {
        if self.scanning {
            return;
        }
        if let Some(path) = self.selected.as_ref().or(self.current.as_ref()) {
            if path.exists() {
                cx.reveal_path(path);
            } else {
                self.status = "This path no longer exists. Scan again to refresh.".into();
                cx.notify();
            }
        }
    }
}

fn color(node: &RenderNode) -> u32 {
    if node.is_dir {
        return 0x69b5a8;
    }
    match node
        .path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "svg" => 0xd98e93,
        "mp4" | "mov" | "mkv" | "avi" | "webm" => 0x9d91ca,
        "mp3" | "wav" | "flac" | "m4a" | "ogg" => 0xc48ab5,
        "zip" | "gz" | "tar" | "7z" | "rar" | "zst" => 0xd9a45f,
        "rs" | "ts" | "js" | "py" | "go" | "cpp" | "swift" | "html" | "css" => 0x75a8c9,
        "pdf" | "txt" | "md" | "docx" | "xlsx" => 0x91b980,
        "json" | "csv" | "db" | "sqlite" | "toml" | "yaml" => 0x76b8bd,
        _ => 0xb4aa96,
    }
}

impl Render for SpaceInsight {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let can_up = !self.scanning && self.current != self.root;
        let has_path = self.current.is_some() && !self.scanning;
        let tree = self.tree.clone();
        let current = self.current.clone();
        let progress = self.progress.clone();
        let scan_root = self.scan_root.clone();
        let scanning = self.scanning;
        let selected = self.selected.clone();
        let hit_map = Rc::new(RefCell::new((
            Bounds::<Pixels>::default(),
            Vec::<RenderNode>::new(),
        )));
        let paint_hits = hit_map.clone();
        let click_hits = hit_map.clone();
        let has_data = self.scanning || self.tree.is_some();
        let size_value = if self.scanning {
            self.progress
                .as_ref()
                .map(|p| bytes(p.total_size))
                .unwrap_or_default()
        } else {
            self.tree
                .as_ref()
                .and_then(|t| {
                    self.current
                        .as_ref()
                        .and_then(|p| t.get_node(p))
                        .and_then(|id| t.get_arena().get(id))
                        .map(|n| bytes(n.get().cumulative_size))
                })
                .unwrap_or_default()
        };
        let size_label = format!("{} · {}", spaceinsight::crawler::size_basis(), size_value);
        let mut crumbs = div()
            .flex()
            .gap_2()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .text_sm();
        if self.scanning {
            crumbs = crumbs.child(
                self.scan_root
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            );
        } else if let (Some(root), Some(current)) = (&self.root, &self.current) {
            let mut paths = vec![root.clone()];
            if let Ok(relative) = current.strip_prefix(root) {
                let mut path = root.clone();
                for component in relative.components() {
                    path.push(component);
                    paths.push(path.clone());
                }
            }
            for (i, path) in paths.into_iter().enumerate() {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                crumbs = crumbs.child(
                    div()
                        .id(("crumb", i))
                        .cursor_pointer()
                        .child(if i == 0 { name } else { format!("› {name}") })
                        .on_click(
                            cx.listener(move |view, _, _, cx| view.navigate(path.clone(), cx)),
                        ),
                );
            }
        } else {
            crumbs = crumbs.child("Choose a folder to explore");
        }
        div().size_full().flex().flex_col().bg(rgb(0xf4f6f8)).text_color(rgb(0x263849))
            .font_family(if cfg!(target_os="windows") { "Segoe UI" } else if cfg!(target_os="macos") { ".SystemUIFont" } else { "sans-serif" })
            .track_focus(&self.focus).key_context("SpaceInsight")
            .on_action(cx.listener(Self::start_scan)).on_action(cx.listener(Self::choose_folder)).on_action(cx.listener(Self::up))
            .child(div().flex().items_center().gap_3().p_4().bg(rgb(0xffffff)).border_b_1().border_color(rgb(0xdce2e8))
                .child(div().text_lg().font_weight(FontWeight::BOLD).child("SpaceInsight"))
                .child(self.input.clone())
                .child(button("browse", "Choose folder", !scanning).on_click(cx.listener(|view, _, window, cx| view.choose_folder(&ChooseFolder, window, cx))))
                .child(button("scan", if scanning { "Scanning…" } else { "Scan" }, !scanning).bg(rgb(0x176b62)).text_color(rgb(0xffffff))
                    .on_click(cx.listener(|view, _, window, cx| view.start_scan(&Scan, window, cx)))))
            .child(div().flex().items_center().gap_3().px_4().py_3()
                .child(button("up", "↑ Up", can_up).on_click(cx.listener(|view, _, window, cx| view.up(&Up, window, cx))))
                .child(crumbs).child(div().font_weight(FontWeight::BOLD).child(size_label))
                .child(button("copy", "Copy path", has_path).on_click(cx.listener(|view, _, _, cx| view.copy_path(cx))))
                .child(button("reveal", "Show in files", has_path).on_click(cx.listener(|view, _, _, cx| view.reveal(cx)))))
            .child(div().id("visualization").relative().flex_1().min_h_0().mx_4().mb_3().overflow_hidden().rounded_lg().bg(rgb(0xe9eef2))
                .on_mouse_down(MouseButton::Left, cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                    window.focus(&view.focus);
                    if view.scanning { return; }
                    let hits = click_hits.borrow();
                    let position = event.position - hits.0.origin;
                    if let Some(node) = hits.1.iter().find(|node| {
                        let r = node.outer_rect; let x = f32::from(position.x); let y = f32::from(position.y);
                        x >= r.x && x < r.x+r.width && y >= r.y && y < r.y+r.height
                    }) {
                        view.selected = Some(node.path.clone());
                        view.status = format!("{} · {}", node.path.display(), bytes(node.size));
                        if node.is_dir { view.navigate(node.path.clone(), cx); }
                        cx.notify();
                    }
                }))
                .on_mouse_down(MouseButton::Right, cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                    window.focus(&view.focus);
                    if view.scanning { return; }
                    let hits = hit_map.borrow(); let position = event.position - hits.0.origin;
                    if let Some(node) = hits.1.iter().find(|node| { let r=node.outer_rect; let x=f32::from(position.x); let y=f32::from(position.y); x>=r.x && x<r.x+r.width && y>=r.y && y<r.y+r.height }) {
                        view.selected=Some(node.path.clone()); view.status=format!("{} · {} · Use Copy path or Show in files", node.path.display(), bytes(node.size)); cx.notify();
                    }
                }))
                .when(has_data, |el| el.child(canvas(move |bounds, _, _| {
                    let rect = Rect::new(0., 0., f32::from(bounds.size.width), f32::from(bounds.size.height));
                    let nodes = if scanning {
                        progress.as_ref().zip(scan_root.as_ref()).map(|(p, root)| {
                            let items: Vec<_> = p.top_level_preview.iter().enumerate().filter(|(_,p)|p.size>0).map(|(index,p)|TreemapItem {size:p.size,index}).collect();
                            SquarifiedTreemap::layout(&items, rect).into_iter().map(|r| { let p=&p.top_level_preview[r.index]; RenderNode { path:root.join(&p.name),name:p.name.clone(),size:p.size,is_dir:p.is_dir,outer_rect:r.rect,content_rect:None,children:vec![],stable_id:r.index as u64,is_aggregate:false,aggregate_count:0 } }).collect()
                        }).unwrap_or_default()
                    } else {
                        tree.as_ref().zip(current.as_ref()).and_then(|(t,p)|t.get_node(p).map(|id|build_render_tree(t,id,rect,&ExpansionState::default(),1,&HashSet::new()))).unwrap_or_default()
                    };
                    *paint_hits.borrow_mut() = (bounds, nodes.clone()); nodes
                }, move |bounds, nodes, window, cx| {
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        if nodes.is_empty() {
                            let text: SharedString = if scanning { "Reading directory entries…" } else { "No non-empty files in this folder" }.into();
                            let _ = window.text_system().shape_line(text, px(16.), &[TextRun {len:if scanning {"Reading directory entries…".len()}else{"No non-empty files in this folder".len()},font:window.text_style().font(),color:rgb(0x536779).into(),background_color:None,underline:None,strikethrough:None}],None).paint(bounds.origin+point(px(24.),px(24.)),px(24.),window,cx);
                        }
                        for node in nodes {
                            let r = node.outer_rect;
                            if r.width < 0.5 || r.height < 0.5 { continue; }
                            let tile = Bounds::new(bounds.origin+point(px(r.x+1.),px(r.y+1.)),size(px((r.width-2.).max(0.)),px((r.height-2.).max(0.))));
                            window.paint_quad(fill(tile, rgb(if selected.as_ref()==Some(&node.path) {0x429e91}else{color(&node)})));
                            if r.width > 60. && r.height > 42. {
                                window.with_content_mask(Some(ContentMask {bounds:tile}), |window| {
                                    for (i,text) in [format!("{}{}",if node.is_dir {"▸ "}else{""},node.name),bytes(node.size)].into_iter().enumerate() {
                                        let text: SharedString=text.into();
                                        let run=TextRun {len:text.len(),font:window.text_style().font(),color:rgb(0x16362f).into(),background_color:None,underline:None,strikethrough:None};
                                        let line=window.text_system().shape_line(text,px(if i==0 {13.}else{11.}),&[run],None);
                                        let _=line.paint(tile.origin+point(px(10.),px(8.+i as f32*20.)),px(18.),window,cx);
                                    }
                                });
                            }
                        }
                    });
                }).size_full()))
                .when(!has_data, |el| el.child(div().size_full().flex().flex_col().items_center().justify_center().gap_3()
                    .child(div().text_2xl().child("See what is using your space"))
                    .child("Choose a folder, then scan to explore its contents."))))
            .child(div().flex().justify_between().px_4().pb_3().text_xs().text_color(rgb(0x617487))
                .child(self.status.clone()).child(if scanning {"Live preview · navigation paused".to_string()}else{format!("{} files · {} folders  |  Click folder to open · Right-click to select", self.files,self.dirs)}))
    }
}

fn main() {
    Application::new().run(|cx| {
        input::bind_keys(cx);
        let modifier = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        cx.bind_keys([
            KeyBinding::new("enter", Scan, Some("SpaceInsight")),
            KeyBinding::new("alt-up", Up, Some("SpaceInsight")),
            KeyBinding::new(&format!("{modifier}-o"), ChooseFolder, Some("SpaceInsight")),
            KeyBinding::new(&format!("{modifier}-q"), Quit, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let initial = std::env::args_os()
            .nth(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(920.), px(640.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("SpaceInsight".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus);
                    SpaceInsight {
                        input: cx.new(|cx| {
                            input::TextInput::new(initial.to_string_lossy().into_owned(), cx)
                        }),
                        tree: None,
                        root: None,
                        current: None,
                        selected: None,
                        progress: None,
                        scan_root: None,
                        scanning: false,
                        files: 0,
                        dirs: 0,
                        status: "Ready".into(),
                        focus,
                        _scan_task: None,
                    }
                })
            },
        )
        .expect("Unable to open SpaceInsight window");
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn sizes_use_decimal_units_like_macos_storage() {
        assert_eq!(super::bytes(1_000_000_000), "1.0 GB");
        assert_eq!(super::bytes(1_000_000), "1.0 MB");
        assert_eq!(super::bytes(999), "999 B");
    }
}
