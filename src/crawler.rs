use dashmap::DashMap;
use jwalk::WalkDir;
use rayon::iter::ParallelBridge;
use rayon::prelude::*;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

const PREVIEW_TOP_LIMIT: usize = 40;
const PROGRESS_EMIT_INTERVAL_MS: u64 = 100;
static INIT_RAYON_POOL: Once = Once::new();

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanPhase {
    Discovering,
    Processing,
}

#[derive(Debug, Clone)]
pub struct ScanProgress {
    pub phase: ScanPhase,
    pub discovered_entries: u64,
    pub processed_entries: u64,
    pub total_entries: Option<u64>,
    pub total_files: u64,
    pub total_dirs: u64,
    pub total_size: u64,
    pub top_level_preview: Vec<ScanTopLevelPreview>,
}

#[derive(Debug, Clone)]
pub struct ScanTopLevelPreview {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

#[derive(Debug, Clone)]
struct PreviewBucket {
    size: u64,
    is_dir: bool,
}

impl ScanProgress {
    pub fn fraction(&self) -> Option<f32> {
        match self.phase {
            ScanPhase::Discovering => None,
            ScanPhase::Processing => {
                let total = self.total_entries?;
                if total == 0 {
                    Some(1.0)
                } else {
                    Some((self.processed_entries as f32 / total as f32).clamp(0.0, 1.0))
                }
            }
        }
    }
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

    fn emit_progress(
        reporter: &Option<Arc<dyn Fn(ScanProgress) + Send + Sync>>,
        progress: ScanProgress,
    ) {
        if let Some(cb) = reporter {
            cb(progress);
        }
    }

    fn preview_snapshot(preview_map: &DashMap<OsString, PreviewBucket>) -> Vec<ScanTopLevelPreview> {
        let mut preview_items: Vec<_> = preview_map
            .iter()
            .map(|entry| {
                let bucket = entry.value();
                ScanTopLevelPreview {
                    name: entry.key().to_string_lossy().to_string(),
                    size: bucket.size,
                    is_dir: bucket.is_dir,
                }
            })
            .collect();
        preview_items.sort_by(|a, b| b.size.cmp(&a.size));
        if preview_items.len() > PREVIEW_TOP_LIMIT {
            preview_items.truncate(PREVIEW_TOP_LIMIT);
        }
        preview_items
    }

    fn should_emit_progress(last_emit_ms: &AtomicU64, elapsed_ms: u64) -> bool {
        let previous = last_emit_ms.load(Ordering::Relaxed);
        if elapsed_ms.saturating_sub(previous) < PROGRESS_EMIT_INTERVAL_MS {
            return false;
        }

        last_emit_ms
            .compare_exchange(previous, elapsed_ms, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    }

    fn processing_parallelism() -> usize {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        (cores * 3).clamp(6, 64)
    }

    fn ensure_high_parallelism() {
        let threads = Self::processing_parallelism();
        INIT_RAYON_POOL.call_once(move || {
            let _ = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build_global();
        });
    }

    fn top_level_name(root_path: &Path, path: &Path) -> Option<(OsString, bool)> {
        let rel = path.strip_prefix(root_path).ok()?;
        let mut components = rel.components();
        let first = components.next()?;
        let has_more = components.next().is_some();
        Some((first.as_os_str().to_os_string(), has_more))
    }

    /// Scan a directory and build a flat list of all files/directories.
    pub fn scan_with_progress<P: AsRef<Path>>(
        &mut self,
        root: P,
        reporter: Option<Arc<dyn Fn(ScanProgress) + Send + Sync>>,
    ) -> (Vec<FileNode>, ScanStats) {
        let root_path = root.as_ref().to_path_buf();
        let reporting_enabled = reporter.is_some();
        let start = Instant::now();
        let top_level_preview: Arc<DashMap<OsString, PreviewBucket>> = Arc::new(DashMap::new());

        // Reset counters
        self.file_count.store(0, Ordering::Relaxed);
        self.dir_count.store(0, Ordering::Relaxed);
        self.total_size.store(0, Ordering::Relaxed);

        Self::ensure_high_parallelism();

        let walker = WalkDir::new(root.as_ref())
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
            .into_iter();

        Self::emit_progress(
            &reporter,
            ScanProgress {
                phase: ScanPhase::Processing,
                discovered_entries: 0,
                processed_entries: 0,
                total_entries: None,
                total_files: 0,
                total_dirs: 0,
                total_size: 0,
                top_level_preview: Vec::new(),
            },
        );

        let discovered_entries = Arc::new(AtomicU64::new(0));
        let processing_last_emit_ms = Arc::new(AtomicU64::new(0));
        let processing_started = Instant::now();
        let reporter_parallel = reporter.clone();
        let discovered_parallel = discovered_entries.clone();
        let file_count = self.file_count.clone();
        let dir_count = self.dir_count.clone();
        let total_size = self.total_size.clone();
        let preview_map = top_level_preview.clone();
        let root_for_workers = root_path.clone();

        let nodes = walker
            .par_bridge()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let discovered_now = discovered_parallel.fetch_add(1, Ordering::Relaxed) + 1;

                let path = entry.path();
                let metadata = entry.metadata().ok()?;

                let size = metadata.len();
                let is_dir = metadata.is_dir();

                if is_dir {
                    dir_count.fetch_add(1, Ordering::Relaxed);
                } else {
                    file_count.fetch_add(1, Ordering::Relaxed);
                    total_size.fetch_add(size, Ordering::Relaxed);
                }

                if reporting_enabled {
                    if let Some((name, nested)) = Self::top_level_name(&root_for_workers, &path) {
                        let bucket_is_dir = nested || is_dir;
                        let mut preview = preview_map
                            .entry(name.clone())
                            .or_insert_with(|| PreviewBucket {
                                size: 0,
                                is_dir: bucket_is_dir,
                            });

                        if bucket_is_dir {
                            preview.is_dir = true;
                        }

                        if !is_dir {
                            preview.size = preview.size.saturating_add(size.max(1));
                        }
                    }

                    if let Some(cb) = reporter_parallel.as_ref() {
                        let elapsed_ms = processing_started.elapsed().as_millis() as u64;
                        if Self::should_emit_progress(&processing_last_emit_ms, elapsed_ms) {
                            cb(ScanProgress {
                                phase: ScanPhase::Processing,
                                discovered_entries: discovered_now,
                                processed_entries: discovered_now,
                                total_entries: None,
                                total_files: file_count.load(Ordering::Relaxed),
                                total_dirs: dir_count.load(Ordering::Relaxed),
                                total_size: total_size.load(Ordering::Relaxed),
                                top_level_preview: Self::preview_snapshot(&preview_map),
                            });
                        }
                    }
                }

                Some(FileNode {
                    path: path.to_path_buf(),
                    size,
                    is_dir,
                })
            })
            .collect::<Vec<_>>();

        let total_entries = discovered_entries.load(Ordering::Relaxed);

        Self::emit_progress(
            &reporter,
            ScanProgress {
                phase: ScanPhase::Processing,
                discovered_entries: total_entries,
                processed_entries: total_entries,
                total_entries: Some(total_entries),
                total_files: self.file_count.load(Ordering::Relaxed),
                total_dirs: self.dir_count.load(Ordering::Relaxed),
                total_size: self.total_size.load(Ordering::Relaxed),
                top_level_preview: Self::preview_snapshot(&top_level_preview),
            },
        );

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
        let mut matched = 0usize;
        const DOCKER_VM_PATH: [&str; 5] = ["Library", "Containers", "com.docker.docker", "Data", "vms"];

        for component in path.components() {
            let Some(part) = component.as_os_str().to_str() else {
                continue;
            };

            if part == DOCKER_VM_PATH[matched] {
                matched += 1;
                if matched == DOCKER_VM_PATH.len() {
                    return true;
                }
            }
        }

        false
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
        let (nodes, stats) = crawler.scan_with_progress(".", None);
        
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
