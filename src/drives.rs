use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::thread;
use std::time::SystemTime;

use crate::cache::IndexedEntry;

const RECENT_FILES_PER_DRIVE: usize = 12;

#[derive(Debug, Clone)]
pub struct Drive {
    pub name: String,
    pub root: PathBuf,
    pub total_size: u64,
    pub volume_total_bytes: Option<u64>,
    pub volume_free_bytes: Option<u64>,
    pub scanned_at: SystemTime,
    pub recent_files: Vec<RecentFile>,
}

#[derive(Debug, Clone)]
pub struct RecentFile {
    pub path: PathBuf,
    pub size: u64,
    pub accessed: SystemTime,
}

#[derive(Debug, Clone, Copy)]
pub struct VolumeCapacity {
    pub total_bytes: u64,
    pub free_bytes: u64,
}

#[cfg(windows)]
pub fn volume_capacity(root: &Path) -> Option<VolumeCapacity> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let path: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free_for_user = 0u64;
    let mut total_bytes = 0u64;
    let mut total_free_bytes = 0u64;
    let succeeded = unsafe {
        GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut free_for_user,
            &mut total_bytes,
            &mut total_free_bytes,
        )
    };
    (succeeded != 0).then_some(VolumeCapacity {
        total_bytes,
        free_bytes: total_free_bytes,
    })
}

#[cfg(not(windows))]
pub fn volume_capacity(_root: &Path) -> Option<VolumeCapacity> {
    None
}

#[derive(Debug, Clone)]
pub struct FsEntry {
    pub id: Option<u64>,
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
    pub accessed: Option<SystemTime>,
    pub modified: Option<SystemTime>,
    pub file_count: u64,
    pub folder_count: u64,
}

pub struct DriveScanResult {
    pub total_size: u64,
    pub recent_files: Vec<RecentFile>,
}

#[derive(Debug, Clone)]
pub struct ScanProgress {
    pub drive_name: String,
    pub units_done: usize,
    pub units_total: usize,
    pub files_scanned: usize,
    pub dirs_scanned: usize,
    pub bytes_found: u64,
}

/// Blocking; call from a background thread. Checks each letter root
/// (`C:\`, `D:\`, ...) since std has no direct "list volumes" API.
#[cfg(windows)]
pub fn discover_drive_roots() -> Vec<(String, PathBuf)> {
    (b'A'..=b'Z')
        .filter_map(|letter| {
            let name = format!("{}:\\", letter as char);
            let root = PathBuf::from(&name);
            std::fs::metadata(&root).ok().map(|_| (name, root))
        })
        .collect()
}

#[cfg(not(windows))]
pub fn discover_drive_roots() -> Vec<(String, PathBuf)> {
    vec![("/".to_string(), PathBuf::from("/"))]
}

/// Recursively sums file sizes and reports progress while walking `path`.
/// Unreadable entries and links are skipped rather than failing the scan.
pub fn scan_drive_with_sink(
    drive_name: &str,
    path: &Path,
    report: impl FnMut(ScanProgress) + Send,
    persist: impl FnMut(Vec<IndexedEntry>) + Send,
) -> DriveScanResult {
    const REPORT_EVERY: usize = 1024;
    const QUEUE_BATCH_SIZE: usize = 512;
    const PERSIST_BATCH_SIZE: usize = 8192;
    const MAX_WORKERS: usize = 4;

    let units: Vec<ScanJob> = match fs::read_dir(path) {
        Ok(entries) => entries
            .enumerate()
            .filter_map(|(index, entry)| {
                let entry = entry.ok()?;
                Some(ScanJob {
                    id: index as u64 + 1,
                    parent_id: 0,
                    depth: 1,
                    path: entry.path(),
                    metadata: entry.metadata().ok()?,
                })
            })
            .collect(),
        Err(_) => {
            return DriveScanResult {
                total_size: 0,
                recent_files: Vec::new(),
            };
        }
    };
    let initial_units = units.len();
    let mut units = units;
    for (index, unit) in units.iter_mut().enumerate() {
        unit.id = index as u64 + 1;
    }
    if initial_units == 0 {
        return DriveScanResult {
            total_size: 0,
            recent_files: Vec::new(),
        };
    }

    let worker_count = thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(2)
        .clamp(1, MAX_WORKERS);
    let next_entry_id = AtomicU64::new(initial_units as u64 + 1);
    let queue = Mutex::new(WorkQueue {
        paths: units.into(),
        outstanding: initial_units,
    });
    let queue_ready = Condvar::new();
    let units_discovered = AtomicUsize::new(initial_units);
    let units_done = AtomicUsize::new(0);
    let files_scanned = AtomicUsize::new(0);
    let dirs_scanned = AtomicUsize::new(0);
    let bytes_found = AtomicU64::new(0);
    let report = Mutex::new(report);
    let persist = Mutex::new(persist);
    let recent_files = Mutex::new(Vec::new());

    thread::scope(|scope| {
        for _ in 0..worker_count {
            let drive_name = drive_name.to_owned();
            let report = &report;
            let queue = &queue;
            let queue_ready = &queue_ready;
            let units_discovered = &units_discovered;
            let units_done = &units_done;
            let next_entry_id = &next_entry_id;
            let files_scanned = &files_scanned;
            let dirs_scanned = &dirs_scanned;
            let bytes_found = &bytes_found;
            let persist = &persist;
            let recent_files = &recent_files;

            scope.spawn(move || {
                let mut worker_recent = Vec::with_capacity(RECENT_FILES_PER_DRIVE);
                let mut worker_batch = Vec::with_capacity(PERSIST_BATCH_SIZE);
                let report_progress = || {
                    let mut callback = report
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    callback(ScanProgress {
                        drive_name: drive_name.clone(),
                        units_done: units_done.load(Ordering::Relaxed),
                        units_total: units_discovered.load(Ordering::Relaxed),
                        files_scanned: files_scanned.load(Ordering::Relaxed),
                        dirs_scanned: dirs_scanned.load(Ordering::Relaxed),
                        bytes_found: bytes_found.load(Ordering::Relaxed),
                    });
                };

                let mut entries_since_report = 0usize;
                loop {
                    let entry_path = {
                        let mut state = queue
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        loop {
                            if let Some(path) = state.paths.pop_front() {
                                break Some(path);
                            }
                            if state.outstanding == 0 {
                                break None;
                            }
                            state = queue_ready
                                .wait(state)
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                        }
                    };
                    let Some(job) = entry_path else {
                        break;
                    };
                    let entry_id = job.id;
                    let parent_id = job.parent_id;
                    let depth = job.depth;
                    let entry_path = job.path;
                    let metadata = job.metadata;
                    let is_link = metadata.file_type().is_symlink() || is_reparse_point(&metadata);
                    let is_dir = metadata.file_type().is_dir() && !is_link;
                    let size = metadata.len();
                    let accessed = metadata.accessed().ok();
                    let modified = metadata.modified().ok();

                    worker_batch.push(IndexedEntry {
                        id: entry_id,
                        parent_id,
                        depth,
                        name: entry_path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        is_dir,
                        is_link,
                        size,
                        accessed,
                        modified,
                    });
                    if worker_batch.len() >= PERSIST_BATCH_SIZE {
                        let batch = std::mem::replace(
                            &mut worker_batch,
                            Vec::with_capacity(PERSIST_BATCH_SIZE),
                        );
                        let mut callback = persist
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        callback(batch);
                    }

                    if is_dir {
                        dirs_scanned.fetch_add(1, Ordering::Relaxed);
                        if let Ok(entries) = fs::read_dir(&entry_path) {
                            let mut batch = Vec::with_capacity(QUEUE_BATCH_SIZE);
                            for entry in entries.flatten() {
                                if let Ok(metadata) = entry.metadata() {
                                    batch.push(ScanJob {
                                        id: next_entry_id.fetch_add(1, Ordering::Relaxed),
                                        parent_id: entry_id,
                                        depth: depth + 1,
                                        path: entry.path(),
                                        metadata,
                                    });
                                }
                                if batch.len() == QUEUE_BATCH_SIZE {
                                    enqueue_paths(queue, queue_ready, units_discovered, &mut batch);
                                }
                            }
                            enqueue_paths(queue, queue_ready, units_discovered, &mut batch);
                        }
                    } else if metadata.file_type().is_file() {
                        files_scanned.fetch_add(1, Ordering::Relaxed);
                        bytes_found.fetch_add(size, Ordering::Relaxed);
                        if let Some(accessed) = accessed {
                            keep_recent_file(&mut worker_recent, &entry_path, size, accessed);
                        }
                    }

                    units_done.fetch_add(1, Ordering::Relaxed);
                    entries_since_report += 1;
                    if entries_since_report >= REPORT_EVERY {
                        report_progress();
                        entries_since_report = 0;
                    }

                    let mut state = queue
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    state.outstanding -= 1;
                    let finished = state.outstanding == 0;
                    drop(state);
                    if finished {
                        queue_ready.notify_all();
                        report_progress();
                    }
                }

                if !worker_batch.is_empty() {
                    let mut callback = persist
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    callback(worker_batch);
                }

                let mut shared_recent = recent_files
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                shared_recent.extend(worker_recent);
                sort_recent_files(&mut shared_recent);
                shared_recent.truncate(RECENT_FILES_PER_DRIVE);
            });
        }
    });

    DriveScanResult {
        total_size: bytes_found.load(Ordering::Relaxed),
        recent_files: recent_files
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    }
}

struct WorkQueue {
    paths: VecDeque<ScanJob>,
    outstanding: usize,
}

struct ScanJob {
    id: u64,
    parent_id: u64,
    depth: u32,
    path: PathBuf,
    metadata: fs::Metadata,
}

fn enqueue_paths(
    queue: &Mutex<WorkQueue>,
    queue_ready: &Condvar,
    units_discovered: &AtomicUsize,
    batch: &mut Vec<ScanJob>,
) {
    if batch.is_empty() {
        return;
    }

    let count = batch.len();
    let mut state = queue
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.paths.extend(batch.drain(..));
    state.outstanding += count;
    units_discovered.fetch_add(count, Ordering::Relaxed);
    drop(state);
    queue_ready.notify_one();
}

fn keep_recent_file(recent: &mut Vec<RecentFile>, path: &Path, size: u64, accessed: SystemTime) {
    if recent.len() == RECENT_FILES_PER_DRIVE
        && recent
            .last()
            .is_some_and(|oldest| oldest.accessed >= accessed)
    {
        return;
    }

    recent.push(RecentFile {
        path: path.to_path_buf(),
        size,
        accessed,
    });
    sort_recent_files(recent);
    recent.truncate(RECENT_FILES_PER_DRIVE);
}

fn sort_recent_files(recent: &mut [RecentFile]) {
    recent.sort_unstable_by(|left, right| right.accessed.cmp(&left.accessed));
}

pub fn list_directory(path: &Path) -> Result<Vec<FsEntry>, String> {
    let entries = fs::read_dir(path).map_err(|error| error.to_string())?;
    let mut result = Vec::new();

    for entry in entries.flatten() {
        let entry_path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&entry_path) else {
            continue;
        };
        let file_type = metadata.file_type();
        let is_link = file_type.is_symlink() || is_reparse_point(&metadata);
        let is_dir = file_type.is_dir() && !is_link;

        result.push(FsEntry {
            id: None,
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry_path,
            is_dir,
            is_link,
            size: if is_dir { 0 } else { metadata.len() },
            accessed: metadata.accessed().ok(),
            modified: metadata.modified().ok(),
            file_count: 0,
            folder_count: 0,
        });
    }

    result.sort_unstable_by(|left, right| {
        right
            .size
            .cmp(&left.size)
            .then_with(|| right.is_dir.cmp(&left.is_dir))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(result)
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn scan_drive_sums_files_and_reports_completed_units() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("dirmap-scan-{unique}"));
        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("one.bin"), b"123").unwrap();
        fs::write(root.join("two.bin"), b"45").unwrap();

        let updates = Mutex::new(Vec::new());
        let persisted = Mutex::new(Vec::new());
        let result = scan_drive_with_sink(
            "test",
            &root,
            |progress| updates.lock().unwrap().push(progress),
            |batch| persisted.lock().unwrap().extend(batch),
        );
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(result.total_size, 5);
        assert_eq!(result.recent_files.len(), 2);
        let persisted = persisted.into_inner().unwrap();
        assert_eq!(persisted.len(), 3);
        let ids: std::collections::HashSet<_> = persisted.iter().map(|entry| entry.id).collect();
        assert_eq!(ids.len(), persisted.len());
        let nested_id = persisted
            .iter()
            .find(|entry| entry.name == "nested")
            .unwrap()
            .id;
        let nested_file = persisted
            .iter()
            .find(|entry| entry.name == "one.bin")
            .unwrap();
        assert_eq!(nested_file.parent_id, nested_id);
        assert!(
            result
                .recent_files
                .iter()
                .all(|file| file.path.starts_with(&root))
        );
        assert!(
            result
                .recent_files
                .windows(2)
                .all(|pair| pair[0].accessed >= pair[1].accessed)
        );
        let updates = updates.into_inner().unwrap();
        let final_progress = updates.last().unwrap();
        assert_eq!(final_progress.units_done, final_progress.units_total);
        assert_eq!(final_progress.units_total, 3);
        assert_eq!(final_progress.files_scanned, 2);
        assert_eq!(final_progress.dirs_scanned, 1);
        assert_eq!(final_progress.bytes_found, 5);
    }

    #[test]
    fn list_directory_sorts_by_size_descending() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("dirmap-list-{unique}"));
        fs::create_dir_all(root.join("folder")).unwrap();
        fs::write(root.join("file.txt"), b"data").unwrap();

        let entries = list_directory(&root).unwrap();
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "file.txt");
        assert!(!entries[0].is_dir);
        assert_eq!(entries[0].size, 4);
        assert_eq!(entries[1].name, "folder");
        assert!(entries[1].is_dir);
    }
}
