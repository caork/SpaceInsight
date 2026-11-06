//! Headless scanner diagnostics: cargo run --release --example scan-summary -- /path
use spaceinsight::{
    crawler::{size_basis, FileCrawler},
    tree::FileTree,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

fn main() {
    let root = PathBuf::from(std::env::args_os().nth(1).unwrap_or_else(|| ".".into()))
        .canonicalize()
        .expect("invalid scan root");
    let latest = Arc::new(Mutex::new(None));
    let report_slot = latest.clone();
    let (nodes, stats) = FileCrawler::new().scan_with_progress(
        &root,
        Some(Arc::new(move |p| *report_slot.lock().unwrap() = Some(p))),
    );
    let mut tree = FileTree::new(&root);
    for node in &nodes {
        tree.upsert_node(node.path.clone(), node.size, node.is_dir);
    }
    tree.calculate_sizes();
    assert_eq!(
        tree.total_size(),
        stats.total_size,
        "tree and scanner disagree"
    );
    let latest = latest.lock().unwrap();
    let latest = latest.as_ref().unwrap();
    assert_eq!(
        latest.total_size, stats.total_size,
        "progress and completion disagree"
    );
    println!(
        "{} bytes: {} ({:.2} GB), {} files, {} directories, {} unreadable entries, {} ms",
        size_basis(),
        stats.total_size,
        stats.total_size as f64 / 1e9,
        stats.total_files,
        stats.total_dirs,
        stats.unreadable_entries,
        stats.duration_ms
    );
    for item in &latest.top_level_preview {
        println!("{:>16}  {}", item.size, item.name);
    }
    #[cfg(target_os = "macos")]
    if root == std::path::Path::new("/") {
        assert!(
            !nodes.iter().any(|n| ["/System/Volumes", "/Volumes", "/dev"]
                .iter()
                .any(|prefix| n.path.starts_with(prefix)))
        );
        println!(
            "Verified: no secondary APFS namespace, external volume, or device paths in root scan."
        );
    }
}
