use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::SystemTime;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode};
use image::RgbaImage;
use ratatui::layout::Size;
use ratatui_image::Resize;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;

use crate::consent;
use crate::ui::{self, Tui};
use open_source_core::cache::{CachedDrive, CachedEntry, LocalIndex};
use open_source_core::drives::{self, Drive, FsEntry, RecentFile, ScanProgress};
use open_source_core::file_actions::{self, RemovalMode};

const LOGO_BYTES: &[u8] = include_bytes!("../castronstemlogo.png");
const TREE_ICON_BYTES: &[u8] = include_bytes!("../TreeMapicon.png");

/// Widest the logo is allowed to render, in terminal columns.
const LOGO_MAX_COLS: u32 = 20;

/// A decoded logo ready for per-pixel halfblock rendering: 1 pixel per column,
/// 2 pixels per row (top/bottom half of each cell).
pub struct Logo {
    pub image: RgbaImage,
    pub cols: u16,
    pub rows: u16,
}

/// Messages sent from the background scan thread back to the UI thread.
pub enum AppEvent {
    IndexOpened {
        index_path: Option<PathBuf>,
        drives: Option<Vec<Drive>>,
        error: Option<String>,
    },
    DirectorySummaryReady {
        root: PathBuf,
    },
    DirectorySummaryFailed {
        error: String,
    },
    DriveCount(usize),
    Progress(ScanProgress),
    DriveScanned(Drive),
    DirectoryLoaded {
        request_id: u64,
        root: PathBuf,
        path: PathBuf,
        parent_id: Option<u64>,
        entries: Vec<FsEntry>,
        error: Option<String>,
    },
    DirectoryParentResolved {
        request_id: u64,
        parent_id: u64,
    },
    DirectoryStatsLoaded {
        request_id: u64,
        entries: Vec<CachedEntry>,
    },
    DirectoryStatsFinished {
        request_id: u64,
    },
    DirectoryStatsFailed {
        request_id: u64,
        error: String,
    },
    TrashFinished {
        request_id: u64,
        root: PathBuf,
        parent_path: PathBuf,
        parent_id: Option<u64>,
        target_path: PathBuf,
        name: String,
        permanent: bool,
        result: Result<(u64, Option<String>), String>,
    },
    Finished {
        cache_error: Option<String>,
    },
}

pub enum AppState {
    Onboarding {
        intro_frame: usize,
        agreed: bool,
        scroll: u16,
        error: Option<String>,
    },
    Loading {
        spinner_frame: usize,
        started_at: Instant,
        scanned: usize,
        total: usize,
        cache_loading: bool,
        progress_ratio: f64,
        progress: Option<ScanProgress>,
    },
    Ready {
        drives: Vec<Drive>,
        drive_root: Option<PathBuf>,
        path: Option<PathBuf>,
        parent_id: Option<u64>,
        parent_stack: Vec<(PathBuf, Option<u64>)>,
        entries: Vec<FsEntry>,
        selected: usize,
        size_view: bool,
        listing: bool,
        stats_pending: bool,
        stats_total: usize,
        stats_done: usize,
        terms_open: bool,
        terms_scroll: u16,
        pending_trash: Option<PendingTrash>,
        action_notice: Option<String>,
        action_frame: u64,
        error: Option<String>,
        request_id: u64,
    },
}

#[derive(Clone)]
pub struct PendingTrash {
    pub root: PathBuf,
    pub parent_path: PathBuf,
    pub parent_id: Option<u64>,
    pub target_path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub step: TrashStep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrashStep {
    Choose,
    ConfirmPermanent,
    MovingToRecycleBin,
    DeletingPermanently,
}

pub struct App {
    pub state: AppState,
    pub logo: Logo,
    pub tree_icon: Option<Protocol>,
    events: Receiver<AppEvent>,
    sender: Sender<AppEvent>,
    index_path: Option<PathBuf>,
    storage_error: Option<String>,
    drives: Vec<Drive>,
}

impl App {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let logo = load_logo();
        let (accepted, consent_error) = match consent::terms_accepted() {
            Ok(accepted) => (accepted, None),
            Err(error) => (false, Some(error)),
        };
        let mut app = App {
            state: AppState::Onboarding {
                intro_frame: 0,
                agreed: false,
                scroll: 0,
                error: consent_error,
            },
            logo,
            tree_icon: None,
            events: rx,
            sender: tx,
            index_path: None,
            storage_error: None,
            drives: Vec::new(),
        };
        if accepted {
            app.open_after_terms();
        }
        app
    }

    fn open_after_terms(&mut self) {
        self.state = AppState::Loading {
            spinner_frame: 0,
            started_at: Instant::now(),
            scanned: 0,
            total: 0,
            cache_loading: true,
            progress_ratio: 0.0,
            progress: None,
        };
        self.tree_icon = load_tree_icon();
        let sender = self.sender.clone();
        thread::spawn(move || match LocalIndex::open_default() {
            Ok(mut index) => {
                let index_path = Some(index.path().to_path_buf());
                let (drives, error) = match index.load_snapshot() {
                    Ok(snapshot) => (
                        snapshot.map(|drives| drives.into_iter().map(drive_from_cache).collect()),
                        None,
                    ),
                    Err(error) => (None, Some(error)),
                };
                let _ = sender.send(AppEvent::IndexOpened {
                    index_path,
                    drives,
                    error,
                });

                let event_sender = sender.clone();
                if let Err(error) = index.ensure_directory_summaries_with_callback(|root| {
                    let _ = event_sender.send(AppEvent::DirectorySummaryReady { root });
                }) {
                    let _ = sender.send(AppEvent::DirectorySummaryFailed { error });
                }
            }
            Err(error) => {
                let _ = sender.send(AppEvent::IndexOpened {
                    index_path: None,
                    drives: None,
                    error: Some(error),
                });
            }
        });
    }

    pub fn run(mut self, terminal: &mut Tui) -> io::Result<()> {
        loop {
            terminal
                .draw(|frame| ui::draw(frame, &self.state, &self.logo, self.tree_icon.as_ref()))?;

            while let Ok(event) = self.events.try_recv() {
                match event {
                    AppEvent::IndexOpened {
                        index_path,
                        drives,
                        error,
                    } => {
                        self.index_path = index_path;
                        self.storage_error = error.clone();
                        if let Some(drives) = drives {
                            self.state = AppState::Ready {
                                drives,
                                drive_root: None,
                                path: None,
                                parent_id: None,
                                parent_stack: Vec::new(),
                                entries: Vec::new(),
                                selected: 0,
                                size_view: false,
                                listing: false,
                                stats_pending: false,
                                stats_total: 0,
                                stats_done: 0,
                                terms_open: false,
                                terms_scroll: 0,
                                pending_trash: None,
                                action_notice: None,
                                action_frame: 0,
                                error,
                                request_id: 0,
                            };
                        } else {
                            self.start_scan();
                        }
                    }
                    AppEvent::DriveCount(total) => {
                        if let AppState::Loading {
                            total: t,
                            cache_loading,
                            ..
                        } = &mut self.state
                        {
                            *t = total;
                            *cache_loading = false;
                        }
                    }
                    AppEvent::Progress(progress) => {
                        if let AppState::Loading {
                            progress: current,
                            progress_ratio,
                            ..
                        } = &mut self.state
                        {
                            if progress.units_total > 0 {
                                let observed =
                                    progress.units_done as f64 / progress.units_total as f64;
                                *progress_ratio = progress_ratio.max(observed.min(0.95));
                            }
                            *current = Some(progress);
                        }
                    }
                    AppEvent::DriveScanned(drive) => {
                        if let Some(existing) = self
                            .drives
                            .iter_mut()
                            .find(|existing| existing.root == drive.root)
                        {
                            *existing = drive;
                        } else {
                            self.drives.push(drive);
                        }
                        if let AppState::Loading {
                            scanned,
                            progress,
                            progress_ratio,
                            ..
                        } = &mut self.state
                        {
                            *scanned += 1;
                            *progress = None;
                            *progress_ratio = 0.0;
                        }
                    }
                    AppEvent::Finished { cache_error } => {
                        self.state = AppState::Ready {
                            drives: std::mem::take(&mut self.drives),
                            drive_root: None,
                            path: None,
                            parent_id: None,
                            parent_stack: Vec::new(),
                            entries: Vec::new(),
                            selected: 0,
                            size_view: false,
                            listing: false,
                            stats_pending: false,
                            stats_total: 0,
                            stats_done: 0,
                            terms_open: false,
                            terms_scroll: 0,
                            pending_trash: None,
                            action_notice: None,
                            action_frame: 0,
                            error: cache_error,
                            request_id: 0,
                        };
                    }
                    AppEvent::DirectoryLoaded {
                        request_id,
                        root,
                        path,
                        parent_id,
                        entries,
                        error,
                    } => {
                        if let AppState::Ready {
                            drive_root,
                            path: current_path,
                            parent_id: current_parent_id,
                            entries: current_entries,
                            selected,
                            listing,
                            stats_pending,
                            stats_total,
                            stats_done,
                            error: current_error,
                            request_id: current_request,
                            ..
                        } = &mut self.state
                        {
                            if *current_request == request_id {
                                *drive_root = Some(root);
                                *current_path = Some(path);
                                *current_parent_id = parent_id;
                                *current_entries = entries;
                                *selected = 0;
                                *listing = false;
                                *stats_total = current_entries
                                    .iter()
                                    .filter(|entry| entry.is_dir && !entry.is_link)
                                    .count();
                                *stats_done = 0;
                                *stats_pending &= *stats_total > 0;
                                *current_error = error;
                            }
                        }
                    }
                    AppEvent::DirectoryStatsLoaded {
                        request_id,
                        entries: cached_entries,
                    } => {
                        if let AppState::Ready {
                            entries,
                            selected,
                            stats_done,
                            stats_total,
                            request_id: current_request,
                            ..
                        } = &mut self.state
                        {
                            if *current_request == request_id {
                                let selected_name =
                                    entries.get(*selected).map(|entry| entry.name.clone());
                                let mut cached_by_name: HashMap<String, CachedEntry> =
                                    cached_entries
                                        .into_iter()
                                        .map(|entry| (entry.name.clone(), entry))
                                        .collect();
                                let mut completed_folders = 0;
                                for entry in entries.iter_mut() {
                                    if let Some(cached) = cached_by_name.remove(&entry.name) {
                                        if entry.is_dir && !entry.is_link && entry.id.is_none() {
                                            completed_folders += 1;
                                        }
                                        entry.id = Some(cached.id);
                                        entry.size = cached.size;
                                        entry.accessed = cached.accessed;
                                        entry.modified = cached.modified;
                                        entry.file_count = cached.file_count;
                                        entry.folder_count = cached.folder_count;
                                    }
                                }
                                *stats_done = stats_done
                                    .saturating_add(completed_folders)
                                    .min(*stats_total);
                                entries.sort_unstable_by(|left, right| {
                                    right
                                        .size
                                        .cmp(&left.size)
                                        .then_with(|| right.is_dir.cmp(&left.is_dir))
                                        .then_with(|| {
                                            left.name.to_lowercase().cmp(&right.name.to_lowercase())
                                        })
                                });
                                *selected = selected_name
                                    .and_then(|name| {
                                        entries.iter().position(|entry| entry.name == name)
                                    })
                                    .unwrap_or_else(|| {
                                        (*selected).min(entries.len().saturating_sub(1))
                                    });
                            }
                        }
                    }
                    AppEvent::DirectoryStatsFinished { request_id } => {
                        if let AppState::Ready {
                            stats_pending,
                            stats_done,
                            stats_total,
                            request_id: current_request,
                            ..
                        } = &mut self.state
                        {
                            if *current_request == request_id {
                                *stats_pending = false;
                                *stats_done = *stats_total;
                            }
                        }
                    }
                    AppEvent::DirectoryParentResolved {
                        request_id,
                        parent_id,
                    } => {
                        if let AppState::Ready {
                            parent_id: current_parent_id,
                            request_id: current_request,
                            ..
                        } = &mut self.state
                        {
                            if *current_request == request_id {
                                *current_parent_id = Some(parent_id);
                            }
                        }
                    }
                    AppEvent::DirectoryStatsFailed { request_id, error } => {
                        if let AppState::Ready {
                            request_id: current_request,
                            error: current_error,
                            stats_pending,
                            ..
                        } = &mut self.state
                        {
                            if *current_request == request_id {
                                *stats_pending = false;
                                *current_error =
                                    Some(format!("Folder totals unavailable: {error}"));
                            }
                        }
                    }
                    AppEvent::DirectorySummaryReady { root } => {
                        let current_folder = if let AppState::Ready {
                            drive_root,
                            path,
                            parent_id,
                            ..
                        } = &mut self.state
                        {
                            drive_root
                                .as_ref()
                                .filter(|current_root| **current_root == root)
                                .and_then(|_| path.clone())
                                .map(|path| (root, path, *parent_id))
                        } else {
                            None
                        };
                        if let Some((root, path, parent_id)) = current_folder {
                            self.request_listing(root, path, parent_id);
                        }
                    }
                    AppEvent::DirectorySummaryFailed { error } => {
                        if let AppState::Ready {
                            error: current_error,
                            ..
                        } = &mut self.state
                        {
                            *current_error =
                                Some(format!("Folder totals could not be prepared: {error}"));
                        }
                    }
                    AppEvent::TrashFinished {
                        request_id,
                        root,
                        parent_path,
                        parent_id,
                        target_path,
                        name,
                        permanent,
                        result,
                    } => self.finish_trash(
                        request_id,
                        root,
                        parent_path,
                        parent_id,
                        target_path,
                        name,
                        permanent,
                        result,
                    ),
                }
            }

            if let AppState::Onboarding { intro_frame, .. } = &mut self.state {
                *intro_frame = intro_frame.saturating_add(1).min(32);
            }

            if let AppState::Ready {
                pending_trash: Some(pending),
                action_frame,
                ..
            } = &mut self.state
            {
                if matches!(
                    pending.step,
                    TrashStep::MovingToRecycleBin | TrashStep::DeletingPermanently
                ) {
                    *action_frame = action_frame.wrapping_add(1);
                }
            }

            if let AppState::Loading { spinner_frame, .. } = &mut self.state {
                *spinner_frame = spinner_frame.wrapping_add(1);
            }

            // Short poll timeout keeps the spinner animating while still reacting to input quickly.
            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    if self.handle_key(key.code) {
                        return Ok(());
                    }
                }
            }
        }
    }

    fn handle_key(&mut self, key: KeyCode) -> bool {
        if matches!(key, KeyCode::Char('q')) {
            let operation_running = matches!(
                &self.state,
                AppState::Ready {
                    pending_trash: Some(PendingTrash {
                        step: TrashStep::MovingToRecycleBin | TrashStep::DeletingPermanently,
                        ..
                    }),
                    ..
                }
            );
            return !operation_running;
        }

        let pending_step = match &self.state {
            AppState::Ready {
                pending_trash: Some(pending),
                ..
            } => Some(pending.step),
            _ => None,
        };
        if let Some(step) = pending_step {
            match (step, key) {
                (TrashStep::Choose, KeyCode::Char('r')) => self.start_trash(false),
                (TrashStep::Choose, KeyCode::Char('p')) => {
                    if let AppState::Ready { pending_trash, .. } = &mut self.state {
                        if let Some(pending) = pending_trash {
                            pending.step = TrashStep::ConfirmPermanent;
                        }
                    }
                }
                (
                    TrashStep::Choose,
                    KeyCode::Char('n' | 'N') | KeyCode::Esc | KeyCode::Backspace,
                )
                | (
                    TrashStep::ConfirmPermanent,
                    KeyCode::Char('n' | 'N') | KeyCode::Esc | KeyCode::Backspace,
                ) => {
                    if let AppState::Ready {
                        pending_trash,
                        action_notice,
                        ..
                    } = &mut self.state
                    {
                        *pending_trash = None;
                        *action_notice = Some("File action cancelled".to_string());
                    }
                }
                (TrashStep::ConfirmPermanent, KeyCode::Char('y' | 'Y')) => self.start_trash(true),
                (TrashStep::MovingToRecycleBin | TrashStep::DeletingPermanently, _) => {}
                _ => {}
            }
            return false;
        }

        if let AppState::Onboarding {
            intro_frame,
            agreed,
            scroll,
            error,
        } = &mut self.state
        {
            if *intro_frame < 32 {
                return matches!(key, KeyCode::Esc);
            }
            match key {
                KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => *scroll = scroll.saturating_add(1),
                KeyCode::PageUp => *scroll = scroll.saturating_sub(8),
                KeyCode::PageDown => *scroll = scroll.saturating_add(8),
                KeyCode::Home => *scroll = 0,
                KeyCode::End => *scroll = 64,
                KeyCode::Char(' ') => *agreed = !*agreed,
                KeyCode::Enter if *agreed => match consent::record_terms_acceptance() {
                    Ok(()) => self.open_after_terms(),
                    Err(message) => *error = Some(message),
                },
                KeyCode::Esc => return true,
                _ => {}
            }
            return false;
        }

        if let AppState::Ready {
            terms_open,
            terms_scroll,
            ..
        } = &mut self.state
        {
            if *terms_open {
                match key {
                    KeyCode::Esc | KeyCode::Backspace | KeyCode::Left => *terms_open = false,
                    KeyCode::Up | KeyCode::Char('k') => {
                        *terms_scroll = terms_scroll.saturating_sub(1)
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        *terms_scroll = terms_scroll.saturating_add(1)
                    }
                    KeyCode::PageUp => *terms_scroll = terms_scroll.saturating_sub(8),
                    KeyCode::PageDown => *terms_scroll = terms_scroll.saturating_add(8),
                    KeyCode::Home => *terms_scroll = 0,
                    KeyCode::End => *terms_scroll = 64,
                    _ => {}
                }
                return false;
            }
        }

        if matches!(key, KeyCode::Char('b' | 'B')) {
            if let AppState::Ready { size_view, .. } = &mut self.state {
                *size_view = !*size_view;
            }
            return false;
        }

        if matches!(key, KeyCode::Esc | KeyCode::Backspace | KeyCode::Left) {
            self.navigate_parent();
            return false;
        }

        if matches!(key, KeyCode::Char('r')) {
            if matches!(self.state, AppState::Ready { .. }) {
                self.start_scan();
            }
            return false;
        }

        if matches!(key, KeyCode::Char('t')) {
            if let AppState::Ready {
                terms_open,
                terms_scroll,
                ..
            } = &mut self.state
            {
                *terms_open = true;
                *terms_scroll = 0;
            }
            return false;
        }

        if matches!(key, KeyCode::Char('d' | 'D')) {
            self.request_trash_confirmation();
            return false;
        }

        match key {
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::PageUp => self.move_selection(-10),
            KeyCode::PageDown => self.move_selection(10),
            KeyCode::Home => self.select_row(0),
            KeyCode::End => self.select_last_row(),
            KeyCode::Enter | KeyCode::Right => self.open_selected(),
            _ => {}
        }

        false
    }

    fn request_trash_confirmation(&mut self) {
        if let AppState::Ready {
            drive_root,
            path,
            parent_id,
            entries,
            selected,
            listing,
            pending_trash,
            action_notice,
            ..
        } = &mut self.state
        {
            if *listing {
                *action_notice = Some("Wait for the current folder listing to finish".to_string());
                return;
            }
            let (Some(root), Some(parent_path)) = (drive_root.as_ref(), path.as_ref()) else {
                *action_notice = Some("Open a drive and select a file or folder first".to_string());
                return;
            };
            let Some(entry) = entries.get(*selected) else {
                *action_notice = Some("Select a file or folder first".to_string());
                return;
            };
            if entry.is_link {
                *action_notice = Some("Links cannot be moved from TreeMap".to_string());
                return;
            }
            if file_actions::is_protected_path(&entry.path) {
                *action_notice =
                    Some("Protected system or application paths cannot be moved".to_string());
                return;
            }
            *pending_trash = Some(PendingTrash {
                root: root.clone(),
                parent_path: parent_path.clone(),
                parent_id: *parent_id,
                target_path: entry.path.clone(),
                name: entry.name.clone(),
                is_dir: entry.is_dir,
                size: entry.size,
                step: TrashStep::Choose,
            });
            *action_notice = None;
        }
    }

    fn start_trash(&mut self, permanent: bool) {
        let (request_id, pending) = if let AppState::Ready {
            request_id,
            pending_trash,
            action_frame,
            ..
        } = &mut self.state
        {
            *request_id = request_id.wrapping_add(1);
            *action_frame = 0;
            if let Some(pending) = pending_trash {
                pending.step = if permanent {
                    TrashStep::DeletingPermanently
                } else {
                    TrashStep::MovingToRecycleBin
                };
            }
            (*request_id, pending_trash.clone())
        } else {
            return;
        };
        let Some(pending) = pending else {
            return;
        };

        if let AppState::Ready { action_notice, .. } = &mut self.state {
            *action_notice = Some(if permanent {
                format!("Permanently deleting {}...", pending.name)
            } else {
                format!("Moving {} to Recycle Bin...", pending.name)
            });
        }
        let sender = self.sender.clone();
        let index_path = self.index_path.clone();
        thread::spawn(move || {
            let mode = if permanent {
                RemovalMode::Permanent
            } else {
                RemovalMode::RecycleBin
            };
            let result = match file_actions::remove_path(&pending.target_path, mode) {
                Err(error) => Err(error.to_string()),
                Ok(()) => {
                    let cache_result = index_path
                        .as_deref()
                        .ok_or_else(|| "local index is unavailable".to_string())
                        .and_then(LocalIndex::open_at)
                        .and_then(|mut index| {
                            let parent_id =
                                index.directory_id_for_path(&pending.root, &pending.parent_path)?;
                            index.remove_entry(&pending.root, parent_id, &pending.name)
                        });
                    match cache_result {
                        Ok(removed_size) => Ok((removed_size, None)),
                        Err(error) => Ok((pending.size, Some(error))),
                    }
                }
            };
            let _ = sender.send(AppEvent::TrashFinished {
                request_id,
                root: pending.root,
                parent_path: pending.parent_path,
                parent_id: pending.parent_id,
                target_path: pending.target_path,
                name: pending.name,
                permanent,
                result,
            });
        });
    }

    fn move_selection(&mut self, delta: isize) {
        if let AppState::Ready {
            drives,
            path,
            entries,
            selected,
            listing,
            ..
        } = &mut self.state
        {
            if *listing {
                return;
            }
            let count = if path.is_some() {
                entries.len()
            } else {
                drives.len()
            };
            if count > 0 {
                *selected = selected
                    .saturating_add_signed(delta)
                    .min(count.saturating_sub(1));
            }
        }
    }

    fn select_row(&mut self, row: usize) {
        if let AppState::Ready {
            drives,
            path,
            entries,
            selected,
            ..
        } = &mut self.state
        {
            let count = if path.is_some() {
                entries.len()
            } else {
                drives.len()
            };
            *selected = row.min(count.saturating_sub(1));
        }
    }

    fn select_last_row(&mut self) {
        if let AppState::Ready {
            drives,
            path,
            entries,
            selected,
            ..
        } = &mut self.state
        {
            let count = if path.is_some() {
                entries.len()
            } else {
                drives.len()
            };
            *selected = count.saturating_sub(1);
        }
    }

    fn open_selected(&mut self) {
        let target = match &self.state {
            AppState::Ready {
                drives,
                drive_root: None,
                path: None,
                selected,
                listing: false,
                ..
            } => drives
                .get(*selected)
                .map(|drive| (drive.root.clone(), drive.root.clone(), Some(0), false)),
            AppState::Ready {
                entries,
                drive_root: Some(root),
                path: Some(_),
                selected,
                listing: false,
                ..
            } => entries
                .get(*selected)
                .filter(|entry| entry.is_dir && !entry.is_link)
                .map(|entry| (root.clone(), entry.path.clone(), entry.id, true)),
            _ => None,
        };

        if let Some((root, path, parent_id, push_parent)) = target {
            if push_parent {
                if let AppState::Ready {
                    path: Some(current_path),
                    parent_id: current_parent_id,
                    parent_stack,
                    ..
                } = &mut self.state
                {
                    parent_stack.push((current_path.clone(), *current_parent_id));
                }
            }
            self.request_listing(root, path, parent_id);
        }
    }

    fn navigate_parent(&mut self) {
        let parent = match &mut self.state {
            AppState::Ready {
                drive_root: Some(root),
                path: Some(path),
                parent_stack,
                listing: false,
                ..
            } => {
                if root == path {
                    Some(None)
                } else {
                    parent_stack.pop().map(|(parent_path, parent_id)| {
                        Some((root.clone(), parent_path, parent_id))
                    })
                }
            }
            _ => None,
        };

        match parent {
            Some(Some((root, path, parent_id))) => self.request_listing(root, path, parent_id),
            Some(None) => {
                if let AppState::Ready {
                    drive_root,
                    path,
                    parent_id,
                    parent_stack,
                    entries,
                    selected,
                    listing,
                    error,
                    request_id,
                    ..
                } = &mut self.state
                {
                    *request_id = request_id.wrapping_add(1);
                    *drive_root = None;
                    *path = None;
                    *parent_id = None;
                    parent_stack.clear();
                    entries.clear();
                    *selected = 0;
                    *listing = false;
                    *error = None;
                }
            }
            None => {}
        }
    }

    fn request_listing(&mut self, root: PathBuf, path: PathBuf, parent_id: Option<u64>) {
        let stats_expected = self.index_path.is_some();
        let request_id = if let AppState::Ready {
            listing,
            stats_pending,
            stats_total,
            stats_done,
            error,
            request_id,
            ..
        } = &mut self.state
        {
            *request_id = request_id.wrapping_add(1);
            *listing = true;
            *stats_pending = stats_expected;
            *stats_total = 0;
            *stats_done = 0;
            *error = None;
            *request_id
        } else {
            return;
        };

        let sender = self.sender.clone();
        let index_path = self.index_path.clone();
        thread::spawn(move || {
            let (entries, error) = match drives::list_directory(&path) {
                Ok(entries) => (entries, None),
                Err(error) => (Vec::new(), Some(error)),
            };
            let _ = sender.send(AppEvent::DirectoryLoaded {
                request_id,
                root: root.clone(),
                path: path.clone(),
                parent_id,
                entries,
                error,
            });

            if let Some(index_path) = index_path {
                match LocalIndex::open_reader_at(&index_path) {
                    Ok(index) => {
                        let parent_was_unresolved = parent_id.is_none();
                        let resolved_parent_id = match parent_id {
                            Some(parent_id) => Ok(parent_id),
                            None => index.directory_id_for_path(&root, &path),
                        };
                        match resolved_parent_id {
                            Ok(parent_id) => {
                                if parent_was_unresolved {
                                    let _ = sender.send(AppEvent::DirectoryParentResolved {
                                        request_id,
                                        parent_id,
                                    });
                                }
                                let stats_sender = sender.clone();
                                if let Err(error) = index.list_directory_progressively(
                                    &root,
                                    parent_id,
                                    |entries| {
                                        let _ = stats_sender.send(AppEvent::DirectoryStatsLoaded {
                                            request_id,
                                            entries,
                                        });
                                    },
                                ) {
                                    let _ = sender
                                        .send(AppEvent::DirectoryStatsFailed { request_id, error });
                                } else {
                                    let _ = sender
                                        .send(AppEvent::DirectoryStatsFinished { request_id });
                                }
                            }
                            Err(error) => {
                                let _ = sender
                                    .send(AppEvent::DirectoryStatsFailed { request_id, error });
                            }
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(AppEvent::DirectoryStatsFailed { request_id, error });
                    }
                }
            }
        });
    }

    fn finish_trash(
        &mut self,
        request_id: u64,
        root: PathBuf,
        parent_path: PathBuf,
        parent_id: Option<u64>,
        target_path: PathBuf,
        name: String,
        permanent: bool,
        result: Result<(u64, Option<String>), String>,
    ) {
        let mut refresh = false;
        if let AppState::Ready {
            drives,
            pending_trash,
            action_notice,
            error,
            request_id: current_request,
            ..
        } = &mut self.state
        {
            if *current_request == request_id {
                *pending_trash = None;
                *error = None;
                match result {
                    Ok((removed_size, cache_warning)) => {
                        if let Some(drive) = drives.iter_mut().find(|drive| drive.root == root) {
                            drive.total_size = drive.total_size.saturating_sub(removed_size);
                            drive
                                .recent_files
                                .retain(|file| !file.path.starts_with(&target_path));
                        }
                        *action_notice = Some(match cache_warning {
                            Some(warning) => format!(
                                "{}; cached index needs refresh: {warning}",
                                if permanent {
                                    format!("Permanently deleted {name}")
                                } else {
                                    format!("Moved {name} to Recycle Bin")
                                }
                            ),
                            None if permanent => {
                                format!("Permanently deleted {name} ({removed_size} bytes)")
                            }
                            None => format!("Moved {name} to Recycle Bin ({removed_size} bytes)"),
                        });
                        refresh = true;
                    }
                    Err(message) => {
                        *action_notice = Some(format!("Could not move {name}: {message}"));
                    }
                }
            }
        }
        if refresh {
            self.request_listing(root, parent_path, parent_id);
        }
    }

    fn start_scan(&mut self) {
        if let AppState::Ready { drives, .. } = &mut self.state {
            self.drives = std::mem::take(drives);
        }
        self.state = AppState::Loading {
            spinner_frame: 0,
            started_at: Instant::now(),
            scanned: 0,
            total: 0,
            cache_loading: false,
            progress: None,
            progress_ratio: 0.0,
        };

        let sender = self.sender.clone();
        let index_path = self.index_path.clone();
        let startup_storage_error = self.storage_error.clone();
        thread::spawn(move || {
            let roots = drives::discover_drive_roots();
            let _ = sender.send(AppEvent::DriveCount(roots.len()));
            let (index, mut cache_error) = match index_path {
                Some(path) => match LocalIndex::open_at(&path) {
                    Ok(index) => (Some(Arc::new(Mutex::new(index))), startup_storage_error),
                    Err(error) => (None, Some(error)),
                },
                None => (None, startup_storage_error),
            };

            for (name, root) in roots {
                let _ = sender.send(AppEvent::Progress(ScanProgress {
                    drive_name: name.clone(),
                    units_done: 0,
                    units_total: 1,
                    files_scanned: 0,
                    dirs_scanned: 0,
                    bytes_found: 0,
                }));
                let scan_id = match &index {
                    Some(index) => match index
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .begin_scan(&root, &name)
                    {
                        Ok(scan_id) => Some(scan_id),
                        Err(error) => {
                            cache_error = Some(error);
                            None
                        }
                    },
                    None => None,
                };
                let persist_error = Arc::new(Mutex::new(None::<String>));
                let progress_sender = sender.clone();
                let persist_index = index.clone();
                let persist_error_for_scan = persist_error.clone();
                let scan_result = drives::scan_drive_with_sink(
                    &name,
                    &root,
                    move |progress| {
                        let _ = progress_sender.send(AppEvent::Progress(progress));
                    },
                    move |entries| {
                        let (Some(index), Some(scan_id)) = (&persist_index, scan_id) else {
                            return;
                        };
                        let mut error = persist_error_for_scan
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        if error.is_none() {
                            let result = index
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .insert_entries(scan_id, &entries);
                            if let Err(message) = result {
                                *error = Some(message);
                            }
                        }
                    },
                );

                let persistence_error = persist_error
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take();
                if let Some(error) = persistence_error {
                    cache_error = Some(error);
                } else if let (Some(index), Some(scan_id)) = (&index, scan_id) {
                    let result = index
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .finish_scan(scan_id, scan_result.total_size);
                    if let Err(error) = result {
                        cache_error = Some(error);
                    }
                }

                let capacity = drives::volume_capacity(&root);
                let drive = Drive {
                    volume_total_bytes: capacity.map(|capacity| capacity.total_bytes),
                    volume_free_bytes: capacity.map(|capacity| capacity.free_bytes),
                    name,
                    root,
                    total_size: scan_result.total_size,
                    scanned_at: SystemTime::now(),
                    recent_files: scan_result.recent_files,
                };
                let _ = sender.send(AppEvent::DriveScanned(drive));
            }
            let _ = sender.send(AppEvent::Finished { cache_error });
        });
    }
}

fn drive_from_cache(cached: CachedDrive) -> Drive {
    let capacity = drives::volume_capacity(&cached.root);
    Drive {
        name: cached.name,
        root: cached.root,
        total_size: cached.total_size,
        volume_total_bytes: capacity.map(|capacity| capacity.total_bytes),
        volume_free_bytes: capacity.map(|capacity| capacity.free_bytes),
        scanned_at: cached.scanned_at,
        recent_files: cached
            .recent_files
            .into_iter()
            .map(|file| RecentFile {
                path: file.path,
                size: file.size,
                accessed: file.accessed,
            })
            .collect(),
    }
}

/// Decodes the embedded logo once at startup and downsizes it to a pixel grid
/// that maps directly onto terminal cells (1 pixel wide, 2 pixels tall per
/// cell) - see `ui::render_logo_lines` for how transparency is preserved.
fn load_logo() -> Logo {
    load_embedded_image(
        LOGO_BYTES,
        LOGO_MAX_COLS,
        u32::MAX,
        image::imageops::FilterType::Lanczos3,
    )
}

fn load_tree_icon() -> Option<Protocol> {
    let protocol_type = std::env::var("DIRMAP_IMAGE_PROTOCOL")
        .ok()
        .and_then(|protocol| match protocol.to_ascii_lowercase().as_str() {
            "kitty" => Some(ProtocolType::Kitty),
            "sixel" => Some(ProtocolType::Sixel),
            "iterm2" => Some(ProtocolType::Iterm2),
            _ => None,
        })
        .or_else(|| {
            if std::env::var_os("KITTY_WINDOW_ID").is_some()
                || std::env::var_os("WEZTERM_EXECUTABLE").is_some()
                || std::env::var("TERM_PROGRAM").is_ok_and(|name| name == "WezTerm")
                || std::env::var_os("WT_SESSION").is_some()
            {
                Some(ProtocolType::Kitty)
            } else if std::env::var_os("ITERM_SESSION_ID").is_some()
                || std::env::var("TERM_PROGRAM").is_ok_and(|name| name == "iTerm.app")
            {
                Some(ProtocolType::Iterm2)
            } else {
                None
            }
        })?;
    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(protocol_type);

    let image = image::load_from_memory(TREE_ICON_BYTES).ok()?;
    picker
        .new_protocol(image, Size::new(16, 7), Resize::Fit(None))
        .ok()
}

fn load_embedded_image(
    bytes: &[u8],
    max_cols: u32,
    max_pixel_rows: u32,
    filter: image::imageops::FilterType,
) -> Logo {
    let rgba = image::load_from_memory(bytes)
        .expect("embedded logo should decode")
        .to_rgba8();

    // Trim the transparent padding around the mark so it fills its rendered
    // area instead of floating in empty space.
    let cropped = crop_to_content(&rgba);

    let scale = (max_cols as f32 / cropped.width() as f32)
        .min(max_pixel_rows as f32 / cropped.height() as f32)
        .min(1.0);
    let cols = (cropped.width() as f32 * scale).round().max(1.0) as u32;
    // Terminal half-blocks represent two image rows per cell.
    let max_pixel_rows = max_pixel_rows - max_pixel_rows % 2;
    let mut pixel_height = (cropped.height() as f32 * scale).round() as u32;
    pixel_height = (pixel_height + pixel_height % 2).clamp(2, max_pixel_rows.max(2));

    let resized = image::imageops::resize(&cropped, cols, pixel_height, filter);

    Logo {
        rows: (resized.height() / 2) as u16,
        cols: resized.width() as u16,
        image: resized,
    }
}

/// Crops to the bounding box of pixels with any visible alpha.
fn crop_to_content(img: &RgbaImage) -> RgbaImage {
    let (width, height) = img.dimensions();
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (width, height, 0, 0);

    for (x, y, pixel) in img.enumerate_pixels() {
        if pixel[3] > 10 {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }

    if min_x > max_x || min_y > max_y {
        return img.clone(); // fully transparent; nothing to crop
    }

    image::imageops::crop_imm(img, min_x, min_y, max_x - min_x + 1, max_y - min_y + 1).to_image()
}
