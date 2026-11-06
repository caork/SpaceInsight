use spaceinsight::{
    crawler::{FileCrawler, ScanProgress, ScanStats},
    tree::FileTree,
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
};

pub struct CompletedScan {
    pub root: PathBuf,
    pub tree: FileTree,
    pub stats: ScanStats,
}

pub fn validate_root(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() || !path.is_dir() {
        return Err("Choose an existing, readable directory".into());
    }
    std::fs::read_dir(path).map_err(|e| format!("Unable to read folder: {e}"))?;
    path.canonicalize()
        .map_err(|e| format!("Unable to resolve folder: {e}"))
}

type ProgressSlot = Arc<Mutex<Option<ScanProgress>>>;
type ScanReceiver = mpsc::Receiver<Result<CompletedScan, String>>;

pub fn start(root: PathBuf) -> (ProgressSlot, ScanReceiver) {
    let progress: ProgressSlot = Arc::new(Mutex::new(None));
    let latest = progress.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(|| {
            let reporter = Arc::new(move |p| {
                if let Ok(mut slot) = latest.lock() {
                    *slot = Some(p);
                }
            });
            let (nodes, stats) = FileCrawler::new().scan_with_progress(&root, Some(reporter));
            let mut tree = FileTree::new(&root);
            for node in nodes {
                tree.upsert_node(node.path, node.size, node.is_dir);
            }
            tree.calculate_sizes();
            CompletedScan { root, tree, stats }
        })
        .map_err(|_| "Scanner failed. Try a smaller directory.".to_string());
        let _ = tx.send(result);
    });
    (progress, rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scans_nested_and_empty_folders_and_reports_progress() {
        let root = std::env::temp_dir().join(format!("spaceinsight-gpui-{}", std::process::id()));
        std::fs::create_dir_all(root.join("nested/empty")).unwrap();
        std::fs::write(root.join("nested/你好.txt"), b"1234567").unwrap();
        std::fs::write(root.join("root.txt"), b"123").unwrap();
        let root = validate_root(&root).unwrap();
        let (progress, result) = start(root.clone());
        let scan = result
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
        assert_eq!(scan.tree.total_size(), 10);
        assert_eq!(scan.stats.total_files, 2);
        assert!(scan.tree.get_node(&root.join("nested/empty")).is_some());
        assert_eq!(progress.lock().unwrap().as_ref().unwrap().total_size, 10);
        assert!(validate_root(&root.join("root.txt")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_empty_path() {
        assert!(validate_root(Path::new("")).is_err());
    }
}
