use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

const SCHEMA_VERSION: i64 = 4;

pub struct LocalIndex {
    connection: Connection,
    path: PathBuf,
}

pub struct IndexedEntry {
    pub id: u64,
    pub parent_id: u64,
    pub depth: u32,
    pub name: String,
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
    pub accessed: Option<SystemTime>,
    pub modified: Option<SystemTime>,
}

pub struct CachedEntry {
    pub id: u64,
    pub name: String,
    pub size: u64,
    pub accessed: Option<SystemTime>,
    pub modified: Option<SystemTime>,
    pub file_count: u64,
    pub folder_count: u64,
}

pub struct CachedRecentFile {
    pub path: PathBuf,
    pub size: u64,
    pub accessed: SystemTime,
}

pub struct CachedDrive {
    pub root: PathBuf,
    pub name: String,
    pub total_size: u64,
    pub scanned_at: SystemTime,
    pub recent_files: Vec<CachedRecentFile>,
}

type IndexedChild = (i64, String, bool, bool, i64, Option<i64>, Option<i64>);

impl LocalIndex {
    pub fn open_default() -> Result<Self, String> {
        let path = default_index_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let index = Self::open_at(&path)?;
        index.ensure_wal_mode()?;
        Ok(index)
    }

    pub fn open_at(path: &Path) -> Result<Self, String> {
        let connection = Connection::open(path).map_err(|error| error.to_string())?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(|error| error.to_string())?;
        connection
            .pragma_update(None, "synchronous", "NORMAL")
            .map_err(|error| error.to_string())?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|error| error.to_string())?;
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS metadata (
                    key TEXT PRIMARY KEY,
                    value INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS scans (
                    id INTEGER PRIMARY KEY,
                    root TEXT NOT NULL,
                    name TEXT NOT NULL,
                    total_size INTEGER NOT NULL DEFAULT 0,
                    scanned_at INTEGER,
                    complete INTEGER NOT NULL DEFAULT 0
                );
                CREATE INDEX IF NOT EXISTS scans_latest
                    ON scans(root, complete, id DESC);
                CREATE TABLE IF NOT EXISTS entries (
                    scan_id INTEGER NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
                    entry_id INTEGER NOT NULL,
                    parent_id INTEGER NOT NULL,
                    depth INTEGER NOT NULL DEFAULT 0,
                    name TEXT NOT NULL,
                    is_dir INTEGER NOT NULL,
                    is_link INTEGER NOT NULL,
                    size INTEGER NOT NULL,
                    accessed INTEGER,
                    modified INTEGER,
                    subtree_size INTEGER NOT NULL DEFAULT 0,
                    subtree_files INTEGER NOT NULL DEFAULT 0,
                    subtree_dirs INTEGER NOT NULL DEFAULT 0,
                    subtree_accessed INTEGER,
                    subtree_modified INTEGER,
                    PRIMARY KEY(scan_id, entry_id)
                );
                CREATE TABLE IF NOT EXISTS directory_summaries (
                    scan_id INTEGER PRIMARY KEY REFERENCES scans(id) ON DELETE CASCADE,
                    ready INTEGER NOT NULL DEFAULT 0
                );
                CREATE INDEX IF NOT EXISTS entries_parent
                    ON entries(scan_id, parent_id, is_dir DESC, name COLLATE NOCASE);
                CREATE INDEX IF NOT EXISTS entries_recent
                    ON entries(scan_id, accessed DESC);
                INSERT OR IGNORE INTO metadata(key, value) VALUES ('schema_version', 4);",
            )
            .map_err(|error| error.to_string())?;
        let schema_version: i64 = connection
            .query_row(
                "SELECT value FROM metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if schema_version <= 2 && !entry_column_exists(&connection, "modified")? {
            connection
                .execute_batch("ALTER TABLE entries ADD COLUMN modified INTEGER;")
                .map_err(|error| error.to_string())?;
        }
        if schema_version == 1 || schema_version == 2 {
            connection
                .execute_batch("DROP INDEX IF EXISTS entries_path;")
                .map_err(|error| error.to_string())?;
            if entry_column_exists(&connection, "path")? {
                connection
                    .execute_batch("ALTER TABLE entries DROP COLUMN path;")
                    .map_err(|error| error.to_string())?;
            }
            connection
                .execute(
                    "UPDATE metadata SET value = 3 WHERE key = 'schema_version'",
                    [],
                )
                .map_err(|error| error.to_string())?;
        }
        if schema_version < SCHEMA_VERSION {
            for (column, definition) in [
                ("depth", "INTEGER NOT NULL DEFAULT 0"),
                ("subtree_size", "INTEGER NOT NULL DEFAULT 0"),
                ("subtree_files", "INTEGER NOT NULL DEFAULT 0"),
                ("subtree_dirs", "INTEGER NOT NULL DEFAULT 0"),
                ("subtree_accessed", "INTEGER"),
                ("subtree_modified", "INTEGER"),
            ] {
                if !entry_column_exists(&connection, column)? {
                    connection
                        .execute_batch(&format!(
                            "ALTER TABLE entries ADD COLUMN {column} {definition};"
                        ))
                        .map_err(|error| error.to_string())?;
                }
            }
            connection
                .execute(
                    "UPDATE metadata SET value = ?1 WHERE key = 'schema_version'",
                    [SCHEMA_VERSION],
                )
                .map_err(|error| error.to_string())?;
        } else if schema_version != SCHEMA_VERSION {
            return Err(format!("unsupported index schema version {schema_version}"));
        }
        connection
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS entries_depth
                 ON entries(scan_id, depth, is_dir);",
            )
            .map_err(|error| error.to_string())?;

        Ok(Self {
            connection,
            path: path.to_path_buf(),
        })
    }

    pub fn open_reader_at(path: &Path) -> Result<Self, String> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| error.to_string())?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            connection,
            path: path.to_path_buf(),
        })
    }

    fn ensure_wal_mode(&self) -> Result<(), String> {
        let journal_mode: String = self
            .connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            self.connection
                .pragma_update(None, "journal_mode", "WAL")
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load_snapshot(&self) -> Result<Option<Vec<CachedDrive>>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT s.id, s.root, s.name, s.total_size, s.scanned_at
                 FROM scans s
                 WHERE s.complete = 1
                   AND s.id = (
                       SELECT MAX(latest.id) FROM scans latest
                       WHERE latest.root = s.root AND latest.complete = 1
                   )
                 ORDER BY s.name COLLATE NOCASE",
            )
            .map_err(|error| error.to_string())?;
        let scan_rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        let mut drives = Vec::new();

        for scan in scan_rows {
            let (scan_id, root, name, total_size, scanned_at) =
                scan.map_err(|error| error.to_string())?;
            let mut recent_statement = self
                .connection
                .prepare(
                    "SELECT name, size, accessed FROM entries
                     WHERE scan_id = ?1 AND is_dir = 0 AND accessed IS NOT NULL
                     ORDER BY accessed DESC LIMIT 12",
                )
                .map_err(|error| error.to_string())?;
            let recent_files = recent_statement
                .query_map([scan_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })
                .map_err(|error| error.to_string())?
                .map(|row| {
                    let (path, size, accessed) = row.map_err(|error| error.to_string())?;
                    Ok(CachedRecentFile {
                        path: PathBuf::from(path),
                        size: size.max(0) as u64,
                        accessed: system_time_from_millis(accessed),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;

            drives.push(CachedDrive {
                root: PathBuf::from(root),
                name,
                total_size: total_size.max(0) as u64,
                scanned_at: system_time_from_millis(scanned_at.unwrap_or_default()),
                recent_files,
            });
        }

        if drives.is_empty() {
            Ok(None)
        } else {
            Ok(Some(drives))
        }
    }

    pub fn ensure_directory_summaries_with_callback(
        &mut self,
        mut on_ready: impl FnMut(PathBuf),
    ) -> Result<(), String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT scans.id, scans.root FROM scans
                 LEFT JOIN directory_summaries ON directory_summaries.scan_id = scans.id
                 WHERE scans.complete = 1 AND COALESCE(directory_summaries.ready, 0) = 0
                 ORDER BY scans.id DESC",
            )
            .map_err(|error| error.to_string())?;
        let scans = statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        drop(statement);

        for (scan_id, root) in scans {
            let transaction = self
                .connection
                .transaction()
                .map_err(|error| error.to_string())?;
            rebuild_directory_summaries(&transaction, scan_id)?;
            transaction
                .execute(
                    "INSERT INTO directory_summaries(scan_id, ready) VALUES (?1, 1)
                     ON CONFLICT(scan_id) DO UPDATE SET ready = 1",
                    [scan_id],
                )
                .map_err(|error| error.to_string())?;
            transaction.commit().map_err(|error| error.to_string())?;
            on_ready(PathBuf::from(root));
        }
        Ok(())
    }

    pub fn begin_scan(&mut self, root: &Path, name: &str) -> Result<i64, String> {
        self.connection
            .execute(
                "DELETE FROM scans WHERE root = ?1 AND complete = 0",
                [root.to_string_lossy().as_ref()],
            )
            .map_err(|error| error.to_string())?;
        self.connection
            .execute(
                "INSERT INTO scans(root, name, complete) VALUES (?1, ?2, 0)",
                params![root.to_string_lossy().as_ref(), name],
            )
            .map_err(|error| error.to_string())?;
        Ok(self.connection.last_insert_rowid())
    }

    pub fn insert_entries(&mut self, scan_id: i64, entries: &[IndexedEntry]) -> Result<(), String> {
        if entries.is_empty() {
            return Ok(());
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| error.to_string())?;
        {
            let mut statement = transaction
                .prepare(
                    "INSERT INTO entries(
                                scan_id, entry_id, parent_id, depth, name, is_dir, is_link,
                                size, accessed, modified
                            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                )
                .map_err(|error| error.to_string())?;
            for entry in entries {
                statement
                    .execute(params![
                        scan_id,
                        entry.id.min(i64::MAX as u64) as i64,
                        entry.parent_id.min(i64::MAX as u64) as i64,
                        entry.depth,
                        entry.name,
                        entry.is_dir,
                        entry.is_link,
                        entry.size.min(i64::MAX as u64) as i64,
                        entry.accessed.and_then(system_time_to_millis),
                        entry.modified.and_then(system_time_to_millis),
                    ])
                    .map_err(|error| error.to_string())?;
            }
        }
        transaction.commit().map_err(|error| error.to_string())
    }

    pub fn finish_scan(&mut self, scan_id: i64, total_size: u64) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let root: String = transaction
            .query_row("SELECT root FROM scans WHERE id = ?1", [scan_id], |row| {
                row.get(0)
            })
            .map_err(|error| error.to_string())?;
        rebuild_directory_summaries(&transaction, scan_id)?;
        transaction
            .execute(
                "UPDATE scans SET total_size = ?1, scanned_at = ?2, complete = 1 WHERE id = ?3",
                params![
                    total_size.min(i64::MAX as u64) as i64,
                    system_time_to_millis(SystemTime::now()).unwrap_or_default(),
                    scan_id
                ],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO directory_summaries(scan_id, ready) VALUES (?1, 1)
                 ON CONFLICT(scan_id) DO UPDATE SET ready = 1",
                [scan_id],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM scans WHERE root = ?1 AND id != ?2",
                params![root, scan_id],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())
    }

    pub fn directory_id_for_path(&self, root: &Path, path: &Path) -> Result<u64, String> {
        let scan_id = self
            .connection
            .query_row(
                "SELECT id FROM scans
                 WHERE root = ?1 AND complete = 1
                 ORDER BY id DESC LIMIT 1",
                [root.to_string_lossy().as_ref()],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "No cached snapshot for this drive".to_string())?;
        let relative_path = path
            .strip_prefix(root)
            .map_err(|_| "Directory path is outside the indexed drive".to_string())?;
        let mut parent_id = 0_i64;
        for component in relative_path.components() {
            let Component::Normal(name) = component else {
                continue;
            };
            parent_id = self
                .connection
                .query_row(
                    "SELECT entry_id FROM entries
                     WHERE scan_id = ?1 AND parent_id = ?2 AND name = ?3 COLLATE NOCASE
                       AND is_dir = 1 AND is_link = 0",
                    params![scan_id, parent_id, name.to_string_lossy()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| error.to_string())?
                .ok_or_else(|| {
                    format!(
                        "Directory is not in the cached snapshot: {}",
                        path.display()
                    )
                })?;
        }
        Ok(parent_id.max(0) as u64)
    }

    pub fn list_directory(&self, root: &Path, parent_id: u64) -> Result<Vec<CachedEntry>, String> {
        let scan_id = self
            .connection
            .query_row(
                "SELECT id FROM scans
                 WHERE root = ?1 AND complete = 1
                 ORDER BY id DESC LIMIT 1",
                [root.to_string_lossy().as_ref()],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "No cached snapshot for this drive".to_string())?;
        let summary_ready: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM directory_summaries WHERE scan_id = ?1 AND ready = 1)",
                [scan_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let query = if summary_ready {
            "SELECT entry_id, name,
                    CASE WHEN is_dir = 1 AND is_link = 0 THEN subtree_size ELSE size END,
                    COALESCE(subtree_accessed, accessed),
                    COALESCE(subtree_modified, modified),
                    subtree_files,
                    CASE WHEN is_dir = 1 AND is_link = 0
                        THEN MAX(subtree_dirs - 1, 0) ELSE 0 END
               FROM entries
               WHERE scan_id = ?1 AND parent_id = ?2
               ORDER BY 3 DESC, is_dir DESC, name COLLATE NOCASE"
        } else {
            "WITH RECURSIVE subtree(top_id, entry_id, is_dir, is_link) AS (
                  SELECT entry_id, entry_id, is_dir, is_link FROM entries
                 WHERE scan_id = ?1 AND parent_id = ?2
                 UNION ALL
                  SELECT subtree.top_id, child.entry_id, child.is_dir, child.is_link
                 FROM subtree
                  CROSS JOIN entries child INDEXED BY entries_parent
                  WHERE child.scan_id = ?1 AND child.parent_id = subtree.entry_id
                    AND subtree.is_dir = 1 AND subtree.is_link = 0
               ), aggregates AS (
                 SELECT subtree.top_id,
                       SUM(CASE WHEN entry.is_dir = 0 AND entry.is_link = 0
                              THEN entry.size ELSE 0 END) AS total_size,
                       SUM(CASE WHEN entry.is_dir = 0 AND entry.is_link = 0
                              THEN 1 ELSE 0 END) AS file_count,
                       SUM(CASE WHEN entry.is_dir = 1 AND entry.is_link = 0
                              THEN 1 ELSE 0 END) AS directory_count,
                       MAX(CASE WHEN entry.is_dir = 0 AND entry.is_link = 0
                              THEN entry.accessed END) AS newest_accessed,
                       MAX(CASE WHEN entry.is_dir = 0 AND entry.is_link = 0
                              THEN entry.modified END) AS newest_modified
                 FROM subtree
                 CROSS JOIN entries entry INDEXED BY sqlite_autoindex_entries_1
                 WHERE entry.scan_id = ?1 AND entry.entry_id = subtree.entry_id
                 GROUP BY subtree.top_id
               )
               SELECT entry.entry_id, entry.name,
                    CASE WHEN entry.is_dir = 1 AND entry.is_link = 0
                        THEN COALESCE(aggregates.total_size, 0) ELSE entry.size END,
                    CASE WHEN entry.is_dir = 1 AND entry.is_link = 0
                        THEN COALESCE(aggregates.newest_accessed, entry.accessed) ELSE entry.accessed END,
                    CASE WHEN entry.is_dir = 1 AND entry.is_link = 0
                        THEN COALESCE(aggregates.newest_modified, entry.modified) ELSE entry.modified END,
                    COALESCE(aggregates.file_count, 0),
                    CASE WHEN entry.is_dir = 1 AND entry.is_link = 0
                        THEN MAX(COALESCE(aggregates.directory_count, 0) - 1, 0) ELSE 0 END
               FROM entries entry
               LEFT JOIN aggregates ON aggregates.top_id = entry.entry_id
               WHERE entry.scan_id = ?1 AND entry.parent_id = ?2
               ORDER BY 3 DESC, entry.is_dir DESC, entry.name COLLATE NOCASE"
        };
        let mut statement = self
            .connection
            .prepare(query)
            .map_err(|error| error.to_string())?;
        statement
            .query_map(
                params![scan_id, parent_id.min(i64::MAX as u64) as i64],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )
            .map_err(|error| error.to_string())?
            .map(|row| {
                let (id, name, size, accessed, modified, file_count, folder_count) =
                    row.map_err(|error| error.to_string())?;
                Ok(CachedEntry {
                    id: id.max(0) as u64,
                    name,
                    size: size.max(0) as u64,
                    accessed: accessed.map(system_time_from_millis),
                    modified: modified.map(system_time_from_millis),
                    file_count: file_count.max(0) as u64,
                    folder_count: folder_count.max(0) as u64,
                })
            })
            .collect()
    }

    pub fn list_directory_progressively(
        &self,
        root: &Path,
        parent_id: u64,
        mut on_entries: impl FnMut(Vec<CachedEntry>),
    ) -> Result<(), String> {
        let scan_id = self
            .connection
            .query_row(
                "SELECT id FROM scans
                 WHERE root = ?1 AND complete = 1
                 ORDER BY id DESC LIMIT 1",
                [root.to_string_lossy().as_ref()],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "No cached snapshot for this drive".to_string())?;
        let summary_ready: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM directory_summaries WHERE scan_id = ?1 AND ready = 1)",
                [scan_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if summary_ready {
            on_entries(self.list_directory(root, parent_id)?);
            return Ok(());
        }

        let mut statement = self
            .connection
            .prepare(
                "SELECT entry_id, name, is_dir, is_link, size, accessed, modified
                 FROM entries WHERE scan_id = ?1 AND parent_id = ?2
                 ORDER BY is_dir DESC, name COLLATE NOCASE",
            )
            .map_err(|error| error.to_string())?;
        let children: Vec<IndexedChild> = statement
            .query_map(
                params![scan_id, parent_id.min(i64::MAX as u64) as i64],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                        row.get::<_, bool>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                    ))
                },
            )
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        drop(statement);

        if children.is_empty() {
            return Ok(());
        }

        const MAX_SUMMARY_WORKERS: usize = 4;
        let worker_count = thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(2)
            .clamp(1, MAX_SUMMARY_WORKERS)
            .min(children.len());
        let next_child = AtomicUsize::new(0);
        let (sender, receiver) = mpsc::channel();
        let mut first_error = None;

        thread::scope(|scope| {
            for _ in 0..worker_count {
                let sender = sender.clone();
                let children = &children;
                let next_child = &next_child;
                let database_path = &self.path;
                scope.spawn(move || {
                    let connection = Connection::open_with_flags(
                        database_path,
                        OpenFlags::SQLITE_OPEN_READ_ONLY,
                    )
                    .and_then(|connection| {
                        connection.busy_timeout(Duration::from_secs(5))?;
                        Ok(connection)
                    })
                    .map_err(|error| error.to_string());
                    loop {
                        let child_index = next_child.fetch_add(1, Ordering::Relaxed);
                        let Some(child) = children.get(child_index) else {
                            break;
                        };
                        let result = match &connection {
                            Ok(connection) => summarize_indexed_child(connection, scan_id, child),
                            Err(error) => Err(error.clone()),
                        };
                        if sender.send(result).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(sender);

            for _ in 0..children.len() {
                match receiver.recv() {
                    Ok(Ok(entry)) => on_entries(vec![entry]),
                    Ok(Err(error)) => {
                        first_error.get_or_insert(error);
                    }
                    Err(error) => {
                        first_error.get_or_insert_with(|| error.to_string());
                        break;
                    }
                };
            }
        });

        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    }

    pub fn remove_entry(&mut self, root: &Path, parent_id: u64, name: &str) -> Result<u64, String> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let scan_id: i64 = transaction
            .query_row(
                "SELECT id FROM scans WHERE root = ?1 AND complete = 1 ORDER BY id DESC LIMIT 1",
                [root.to_string_lossy().as_ref()],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let entry_id: i64 = transaction
            .query_row(
                "SELECT entry_id FROM entries
                 WHERE scan_id = ?1 AND parent_id = ?2 AND name = ?3",
                params![scan_id, parent_id.min(i64::MAX as u64) as i64, name],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let removed_size: i64 = transaction
            .query_row(
                "WITH RECURSIVE subtree(entry_id, is_dir, is_link) AS (
                    SELECT entry_id, is_dir, is_link FROM entries
                    WHERE scan_id = ?1 AND entry_id = ?2
                    UNION ALL
                    SELECT child.entry_id, child.is_dir, child.is_link
                    FROM entries child JOIN subtree parent ON child.parent_id = parent.entry_id
                    WHERE child.scan_id = ?1 AND parent.is_dir = 1 AND parent.is_link = 0
                 )
                 SELECT COALESCE(SUM(
                    CASE WHEN entry.is_dir = 0 AND entry.is_link = 0 THEN entry.size ELSE 0 END
                 ), 0)
                 FROM entries entry JOIN subtree ON subtree.entry_id = entry.entry_id
                 WHERE entry.scan_id = ?1",
                params![scan_id, entry_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "WITH RECURSIVE subtree(entry_id, is_dir, is_link) AS (
                    SELECT entry_id, is_dir, is_link FROM entries
                    WHERE scan_id = ?1 AND entry_id = ?2
                    UNION ALL
                    SELECT child.entry_id, child.is_dir, child.is_link
                    FROM entries child JOIN subtree parent ON child.parent_id = parent.entry_id
                    WHERE child.scan_id = ?1 AND parent.is_dir = 1 AND parent.is_link = 0
                 )
                 DELETE FROM entries
                 WHERE scan_id = ?1 AND entry_id IN (SELECT entry_id FROM subtree)",
                params![scan_id, entry_id],
            )
            .map_err(|error| error.to_string())?;
        let total_size: i64 = transaction
            .query_row(
                "SELECT COALESCE(SUM(size), 0) FROM entries
                 WHERE scan_id = ?1 AND is_dir = 0 AND is_link = 0",
                [scan_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "UPDATE scans SET total_size = ?1, scanned_at = ?2 WHERE id = ?3",
                params![
                    total_size,
                    system_time_to_millis(SystemTime::now()).unwrap_or_default(),
                    scan_id
                ],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO directory_summaries(scan_id, ready) VALUES (?1, 0)
                 ON CONFLICT(scan_id) DO UPDATE SET ready = 0",
                [scan_id],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(removed_size.max(0) as u64)
    }
}

fn summarize_indexed_child(
    connection: &Connection,
    scan_id: i64,
    child: &IndexedChild,
) -> Result<CachedEntry, String> {
    let (entry_id, name, is_dir, is_link, size, accessed, modified) = child;
    if *is_dir && !*is_link {
        let (total_size, file_count, directory_count, newest_accessed, newest_modified): (
            i64,
            i64,
            i64,
            Option<i64>,
            Option<i64>,
        ) = connection
            .query_row(
                                "WITH RECURSIVE subtree(entry_id, is_dir, is_link) AS (
                    SELECT entry_id, is_dir, is_link FROM entries
                    WHERE scan_id = ?1 AND entry_id = ?2
                    UNION ALL
                    SELECT child.entry_id, child.is_dir, child.is_link
                                        FROM subtree parent
                                        CROSS JOIN entries child INDEXED BY entries_parent
                                        WHERE child.scan_id = ?1 AND child.parent_id = parent.entry_id
                                            AND parent.is_dir = 1 AND parent.is_link = 0
                 )
                 SELECT COALESCE(SUM(CASE WHEN entry.is_dir = 0 AND entry.is_link = 0
                                          THEN entry.size ELSE 0 END), 0),
                        COALESCE(SUM(CASE WHEN entry.is_dir = 0 AND entry.is_link = 0
                                          THEN 1 ELSE 0 END), 0),
                        COALESCE(SUM(CASE WHEN entry.is_dir = 1 AND entry.is_link = 0
                                          THEN 1 ELSE 0 END), 0),
                        MAX(CASE WHEN entry.is_dir = 0 AND entry.is_link = 0
                                 THEN entry.accessed END),
                        MAX(CASE WHEN entry.is_dir = 0 AND entry.is_link = 0
                                 THEN entry.modified END)
                                 FROM subtree
                                 CROSS JOIN entries entry INDEXED BY sqlite_autoindex_entries_1
                                 WHERE entry.scan_id = ?1 AND entry.entry_id = subtree.entry_id",
                params![scan_id, entry_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .map_err(|error| error.to_string())?;
        Ok(CachedEntry {
            id: (*entry_id).max(0) as u64,
            name: name.clone(),
            size: total_size.max(0) as u64,
            accessed: newest_accessed.map(system_time_from_millis),
            modified: newest_modified.map(system_time_from_millis),
            file_count: file_count.max(0) as u64,
            folder_count: directory_count.saturating_sub(1).max(0) as u64,
        })
    } else {
        Ok(CachedEntry {
            id: (*entry_id).max(0) as u64,
            name: name.clone(),
            size: (*size).max(0) as u64,
            accessed: accessed.map(system_time_from_millis),
            modified: modified.map(system_time_from_millis),
            file_count: u64::from(!*is_dir && !*is_link),
            folder_count: 0,
        })
    }
}

fn rebuild_directory_summaries(
    transaction: &rusqlite::Transaction<'_>,
    scan_id: i64,
) -> Result<(), String> {
    let needs_depth: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM entries WHERE scan_id = ?1 AND depth = 0)",
            [scan_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if needs_depth {
        transaction
            .execute(
                "WITH RECURSIVE tree(entry_id, depth) AS (
                    SELECT entry_id, 1 FROM entries WHERE scan_id = ?1 AND parent_id = 0
                    UNION ALL
                    SELECT child.entry_id, tree.depth + 1
                    FROM entries child JOIN tree ON child.parent_id = tree.entry_id
                    WHERE child.scan_id = ?1
                 )
                 UPDATE entries SET depth = (
                    SELECT tree.depth FROM tree WHERE tree.entry_id = entries.entry_id
                 )
                 WHERE scan_id = ?1 AND depth = 0",
                [scan_id],
            )
            .map_err(|error| error.to_string())?;
    }

    transaction
        .execute(
            "UPDATE entries SET
                subtree_size = CASE WHEN is_dir = 0 AND is_link = 0 THEN size ELSE 0 END,
                subtree_files = CASE WHEN is_dir = 0 AND is_link = 0 THEN 1 ELSE 0 END,
                subtree_dirs = CASE WHEN is_dir = 1 AND is_link = 0 THEN 1 ELSE 0 END,
                subtree_accessed = CASE WHEN is_dir = 0 AND is_link = 0 THEN accessed END,
                subtree_modified = CASE WHEN is_dir = 0 AND is_link = 0 THEN modified END
             WHERE scan_id = ?1",
            [scan_id],
        )
        .map_err(|error| error.to_string())?;
    let max_depth: u32 = transaction
        .query_row(
            "SELECT COALESCE(MAX(depth), 0) FROM entries WHERE scan_id = ?1",
            [scan_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;

    for depth in (1..=max_depth).rev() {
        transaction
            .execute(
                "UPDATE entries AS parent SET
                    subtree_size = COALESCE((
                        SELECT SUM(child.subtree_size) FROM entries child
                        WHERE child.scan_id = parent.scan_id AND child.parent_id = parent.entry_id
                    ), 0),
                    subtree_files = COALESCE((
                        SELECT SUM(child.subtree_files) FROM entries child
                        WHERE child.scan_id = parent.scan_id AND child.parent_id = parent.entry_id
                    ), 0),
                    subtree_dirs = 1 + COALESCE((
                        SELECT SUM(child.subtree_dirs) FROM entries child
                        WHERE child.scan_id = parent.scan_id AND child.parent_id = parent.entry_id
                    ), 0),
                    subtree_accessed = (
                        SELECT MAX(child.subtree_accessed) FROM entries child
                        WHERE child.scan_id = parent.scan_id AND child.parent_id = parent.entry_id
                    ),
                    subtree_modified = (
                        SELECT MAX(child.subtree_modified) FROM entries child
                        WHERE child.scan_id = parent.scan_id AND child.parent_id = parent.entry_id
                    )
                 WHERE parent.scan_id = ?1 AND parent.depth = ?2
                   AND parent.is_dir = 1 AND parent.is_link = 0",
                params![scan_id, depth],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn entry_column_exists(connection: &Connection, name: &str) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM pragma_table_info('entries') WHERE name = ?1
            )",
            [name],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

fn default_index_path() -> Result<PathBuf, String> {
    Ok(data_directory()?.join("index.sqlite3"))
}

/// Returns the local application data directory, honoring `DIRMAP_DATA_DIR`.
pub fn data_directory() -> Result<PathBuf, String> {
    if let Some(directory) = std::env::var_os("DIRMAP_DATA_DIR") {
        return Ok(PathBuf::from(directory));
    }

    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);

    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));

    base.map(|base| base.join("DirMap"))
        .ok_or_else(|| "cannot determine the user data directory".to_string())
}

fn system_time_to_millis(time: SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
}

fn system_time_from_millis(millis: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(millis.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_index_open_does_not_lock_against_active_writer() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("dirmap-reader-lock-{unique}"));
        fs::create_dir_all(&directory).unwrap();
        let database_path = directory.join("index.sqlite3");
        let mut writer = LocalIndex::open_at(&database_path).unwrap();
        writer
            .connection
            .pragma_update(None, "journal_mode", "WAL")
            .unwrap();
        let transaction = writer.connection.transaction().unwrap();
        transaction
            .execute(
                "INSERT INTO scans(root, name, complete) VALUES ('Z:\\', 'Z:\\', 0)",
                [],
            )
            .unwrap();

        let reader = LocalIndex::open_reader_at(&database_path).unwrap();
        let visible_scans: i64 = reader
            .connection
            .query_row("SELECT COUNT(*) FROM scans", [], |row| row.get(0))
            .unwrap();
        assert_eq!(visible_scans, 0);

        drop(reader);
        transaction.rollback().unwrap();
        drop(writer);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn snapshots_are_browsable_and_only_replace_after_completion() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("dirmap-index-{unique}"));
        fs::create_dir_all(&directory).unwrap();
        let database_path = directory.join("index.sqlite3");
        let root = directory.join("R:");
        let mut index = LocalIndex::open_at(&database_path).unwrap();

        let first_scan = index.begin_scan(&root, "R:\\").unwrap();
        index
            .insert_entries(
                first_scan,
                &[
                    IndexedEntry {
                        id: 1,
                        parent_id: 0,
                        depth: 1,
                        name: "folder".to_string(),
                        is_dir: true,
                        is_link: false,
                        size: 0,
                        accessed: None,
                        modified: None,
                    },
                    IndexedEntry {
                        id: 2,
                        parent_id: 1,
                        depth: 2,
                        name: "child.txt".to_string(),
                        is_dir: false,
                        is_link: false,
                        size: 7,
                        accessed: Some(UNIX_EPOCH + Duration::from_secs(10)),
                        modified: Some(UNIX_EPOCH + Duration::from_secs(20)),
                    },
                    IndexedEntry {
                        id: 4,
                        parent_id: 1,
                        depth: 2,
                        name: "subfolder".to_string(),
                        is_dir: true,
                        is_link: false,
                        size: 0,
                        accessed: None,
                        modified: None,
                    },
                    IndexedEntry {
                        id: 5,
                        parent_id: 4,
                        depth: 3,
                        name: "nested.txt".to_string(),
                        is_dir: false,
                        is_link: false,
                        size: 4,
                        accessed: Some(UNIX_EPOCH + Duration::from_secs(30)),
                        modified: Some(UNIX_EPOCH + Duration::from_secs(40)),
                    },
                    IndexedEntry {
                        id: 3,
                        parent_id: 0,
                        depth: 1,
                        name: "root.bin".to_string(),
                        is_dir: false,
                        is_link: false,
                        size: 12,
                        accessed: None,
                        modified: None,
                    },
                ],
            )
            .unwrap();
        assert!(index.load_snapshot().unwrap().is_none());
        index.finish_scan(first_scan, 23).unwrap();
        assert_eq!(
            index
                .directory_id_for_path(&root, &root.join("folder"))
                .unwrap(),
            1
        );

        let snapshot = index.load_snapshot().unwrap().unwrap();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].total_size, 23);
        index
            .connection
            .execute(
                "UPDATE directory_summaries SET ready = 0 WHERE scan_id = ?1",
                [first_scan],
            )
            .unwrap();
        let root_entries = index.list_directory(&root, 0).unwrap();
        assert_eq!(root_entries.len(), 2);
        assert_eq!(root_entries[0].name, "root.bin");
        assert_eq!(root_entries[0].size, 12);
        assert_eq!(root_entries[1].name, "folder");
        assert_eq!(root_entries[1].size, 11);
        assert_eq!(root_entries[1].file_count, 2);
        assert_eq!(root_entries[1].folder_count, 1);
        assert_eq!(
            root_entries[1].accessed,
            Some(UNIX_EPOCH + Duration::from_secs(30))
        );
        assert_eq!(
            root_entries[1].modified,
            Some(UNIX_EPOCH + Duration::from_secs(40))
        );
        let mut streamed_entries = Vec::new();
        index
            .list_directory_progressively(&root, 0, |batch| {
                streamed_entries.extend(batch);
            })
            .unwrap();
        assert_eq!(streamed_entries.len(), 2);
        let streamed_folder = streamed_entries
            .iter()
            .find(|entry| entry.name == "folder")
            .unwrap();
        assert_eq!(streamed_folder.size, 11);
        assert_eq!(streamed_folder.file_count, 2);
        assert_eq!(streamed_folder.folder_count, 1);
        index
            .ensure_directory_summaries_with_callback(|_| {})
            .unwrap();
        let folder_entries = index.list_directory(&root, root_entries[1].id).unwrap();
        assert_eq!(folder_entries.len(), 2);
        assert_eq!(folder_entries[0].name, "child.txt");
        assert_eq!(folder_entries[1].name, "subfolder");
        assert_eq!(folder_entries[1].size, 4);
        assert_eq!(folder_entries[1].file_count, 1);

        let second_scan = index.begin_scan(&root, "R:\\").unwrap();
        index
            .insert_entries(
                second_scan,
                &[IndexedEntry {
                    id: 1,
                    parent_id: 0,
                    depth: 1,
                    name: "new.txt".to_string(),
                    is_dir: false,
                    is_link: false,
                    size: 2,
                    accessed: None,
                    modified: None,
                }],
            )
            .unwrap();
        assert_eq!(index.load_snapshot().unwrap().unwrap()[0].total_size, 23);
        index.finish_scan(second_scan, 2).unwrap();
        assert_eq!(index.load_snapshot().unwrap().unwrap()[0].total_size, 2);
        let refreshed = index.list_directory(&root, 0).unwrap();
        assert_eq!(refreshed.len(), 1);
        assert_eq!(refreshed[0].name, "new.txt");

        drop(index);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn summary_backfill_notifies_newest_drive_first() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("dirmap-summary-order-{unique}"));
        fs::create_dir_all(&directory).unwrap();
        let database_path = directory.join("index.sqlite3");
        let mut index = LocalIndex::open_at(&database_path).unwrap();

        for root in [PathBuf::from("C:\\"), PathBuf::from("H:\\")] {
            let scan_id = index
                .begin_scan(&root, &root.display().to_string())
                .unwrap();
            index.finish_scan(scan_id, 0).unwrap();
        }
        index
            .connection
            .execute("UPDATE directory_summaries SET ready = 0", [])
            .unwrap();

        let mut ready_roots = Vec::new();
        index
            .ensure_directory_summaries_with_callback(|root| ready_roots.push(root))
            .unwrap();

        assert_eq!(
            ready_roots,
            vec![PathBuf::from("H:\\"), PathBuf::from("C:\\")]
        );
        drop(index);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn migrates_v1_full_path_index_to_compact_schema() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("dirmap-v1-{unique}"));
        fs::create_dir_all(&directory).unwrap();
        let database_path = directory.join("index.sqlite3");
        let root = PathBuf::from("S:\\");

        {
            let legacy = Connection::open(&database_path).unwrap();
            legacy
                .execute_batch(
                    "CREATE TABLE metadata(key TEXT PRIMARY KEY, value INTEGER NOT NULL);
                     INSERT INTO metadata(key, value) VALUES ('schema_version', 1);
                     CREATE TABLE scans(
                         id INTEGER PRIMARY KEY, root TEXT NOT NULL, name TEXT NOT NULL,
                         total_size INTEGER NOT NULL DEFAULT 0, scanned_at INTEGER,
                         complete INTEGER NOT NULL DEFAULT 0
                     );
                     CREATE TABLE entries(
                         scan_id INTEGER NOT NULL, path TEXT NOT NULL, entry_id INTEGER NOT NULL,
                         parent_id INTEGER NOT NULL, name TEXT NOT NULL, is_dir INTEGER NOT NULL,
                         is_link INTEGER NOT NULL, size INTEGER NOT NULL, accessed INTEGER,
                         modified INTEGER,
                         PRIMARY KEY(scan_id, entry_id)
                     );
                     CREATE INDEX entries_path ON entries(scan_id, path);
                     INSERT INTO scans(id, root, name, total_size, scanned_at, complete)
                         VALUES (1, 'S:\\', 'S:\\', 4, 1, 1);
                     INSERT INTO entries(
                         scan_id, path, entry_id, parent_id, name, is_dir, is_link, size, accessed
                     ) VALUES (1, 'S:\\folder', 1, 0, 'folder', 1, 0, 0, NULL);
                     INSERT INTO entries(
                         scan_id, path, entry_id, parent_id, name, is_dir, is_link, size, accessed
                     ) VALUES (1, 'S:\\folder\\file.bin', 2, 1, 'file.bin', 0, 0, 4, 10);",
                )
                .unwrap();
        }

        let mut index = LocalIndex::open_at(&database_path).unwrap();
        index
            .ensure_directory_summaries_with_callback(|_| {})
            .unwrap();
        let entries = index.list_directory(&root, 0).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "folder");
        assert_eq!(entries[0].size, 4);
        assert_eq!(entries[0].file_count, 1);
        assert!(entries[0].modified.is_none());
        let schema_version: i64 = index
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(schema_version, SCHEMA_VERSION);
        let columns: Vec<String> = index
            .connection
            .prepare("PRAGMA table_info(entries)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(!columns.iter().any(|column| column == "path"));
        assert!(columns.iter().any(|column| column == "modified"));

        drop(index);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn remove_entry_removes_descendants_and_updates_drive_total() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("dirmap-trash-{unique}"));
        fs::create_dir_all(&directory).unwrap();
        let database_path = directory.join("index.sqlite3");
        let root = PathBuf::from("T:\\");
        let mut index = LocalIndex::open_at(&database_path).unwrap();
        let scan_id = index.begin_scan(&root, "T:\\").unwrap();
        index
            .insert_entries(
                scan_id,
                &[
                    IndexedEntry {
                        id: 1,
                        parent_id: 0,
                        depth: 1,
                        name: "folder".to_string(),
                        is_dir: true,
                        is_link: false,
                        size: 0,
                        accessed: None,
                        modified: None,
                    },
                    IndexedEntry {
                        id: 2,
                        parent_id: 1,
                        depth: 2,
                        name: "nested.bin".to_string(),
                        is_dir: false,
                        is_link: false,
                        size: 7,
                        accessed: None,
                        modified: None,
                    },
                    IndexedEntry {
                        id: 3,
                        parent_id: 0,
                        depth: 1,
                        name: "keep.bin".to_string(),
                        is_dir: false,
                        is_link: false,
                        size: 11,
                        accessed: None,
                        modified: None,
                    },
                ],
            )
            .unwrap();
        index.finish_scan(scan_id, 18).unwrap();

        assert_eq!(index.remove_entry(&root, 0, "folder").unwrap(), 7);
        assert_eq!(index.load_snapshot().unwrap().unwrap()[0].total_size, 11);
        let rows = index.list_directory(&root, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "keep.bin");

        drop(index);
        fs::remove_dir_all(directory).unwrap();
    }
}
