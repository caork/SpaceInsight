use dashmap::DashMap;
use jwalk::WalkDir;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

const ABNORMAL_PATH_MARKERS: [&str; 1] = [
    "/Library/Containers/com.docker.docker/Data/vms",
];

#[derive(Debug, Clone)]
pub struct FileNode {
    pub path: PathBuf,
    pub size: u64,
    pub is_dir: bool,
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct ScanStats {
    pub total_files: u64,
    pub total_dirs: u64,
    pub total_size: u64,
    pub duration_ms: u128,
}

/// High-performance parallel file system crawler
pub struct FileCrawler {
    file_count: Arc<AtomicU64>,
    dir_count: Arc<AtomicU64>,
    total_size: Arc<AtomicU64>,
}

impl FileCrawler {
    pub fn new() -> Self {
        Self {
            file_count: Arc::new(AtomicU64::new(0)),
            dir_count: Arc::new(AtomicU64::new(0)),
            total_size: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Scan a directory and build a flat map of all files/directories
    /// Returns a thread-safe map of path -> FileNode
    pub fn scan<P: AsRef<Path>>(&mut self, root: P) -> (Arc<DashMap<PathBuf, FileNode>>, ScanStats) {
        let start = Instant::now();
        let nodes = Arc::new(DashMap::new());
        
        // Reset counters
        self.file_count.store(0, Ordering::Relaxed);
        self.dir_count.store(0, Ordering::Relaxed);
        self.total_size.store(0, Ordering::Relaxed);

        // Walk directory tree in parallel using jwalk
        let entries: Vec<_> = WalkDir::new(root.as_ref())
            .skip_hidden(false)
            .parallelism(jwalk::Parallelism::RayonDefaultPool {
                busy_timeout: std::time::Duration::from_secs(1),
            })
            .process_read_dir(|_, _, _, children| {
                children.retain(|entry| {
                    entry
                        .as_ref()
                        .map(|dir_entry| !Self::should_skip_path(&dir_entry.path()))
                        .unwrap_or(true)
                });
            })
            .into_iter()
            .filter_map(|entry| entry.ok())
            .collect();

        // Process entries in parallel using rayon
        entries.par_iter().for_each(|entry| {
            let path = entry.path();
            if Self::should_skip_path(&path) {
                return;
            }

            let metadata = entry.metadata().ok();
            
            if let Some(meta) = metadata {
                let size = meta.len();
                let is_dir = meta.is_dir();

                if is_dir {
                    self.dir_count.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.file_count.fetch_add(1, Ordering::Relaxed);
                    self.total_size.fetch_add(size, Ordering::Relaxed);
                }

                let node = FileNode {
                    path: path.to_path_buf(),
                    size,
                    is_dir,
                };

                nodes.insert(path.to_path_buf(), node);
            }
        });

        let duration = start.elapsed();

        let stats = ScanStats {
            total_files: self.file_count.load(Ordering::Relaxed),
            total_dirs: self.dir_count.load(Ordering::Relaxed),
            total_size: self.total_size.load(Ordering::Relaxed),
            duration_ms: duration.as_millis(),
        };

        (nodes, stats)
    }

    fn should_skip_path(path: &Path) -> bool {
        let normalized = path.to_string_lossy().replace('\\', "/");
        ABNORMAL_PATH_MARKERS
            .iter()
            .any(|marker| normalized.contains(marker))
    }
}

impl Default for FileCrawler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crawler_basic() {
        let mut crawler = FileCrawler::new();
        let (nodes, stats) = crawler.scan(".");
        
        assert!(stats.total_files > 0 || stats.total_dirs > 0);
        assert!(!nodes.is_empty());
        println!("Scanned {} files and {} dirs in {}ms", 
                 stats.total_files, stats.total_dirs, stats.duration_ms);
    }

    #[test]
    fn test_skip_abnormal_docker_vm_path() {
        let docker_vm_file = Path::new(
            "/Users/demo/Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw",
        );
        let normal_file = Path::new("/Users/demo/Documents/test.txt");

        assert!(FileCrawler::should_skip_path(docker_vm_file));
        assert!(!FileCrawler::should_skip_path(normal_file));
    }
}
