use std::io::{self, Stdout};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::ExecutableCommand;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph, Wrap,
};
use ratatui::{Frame, Terminal};
use ratatui_image::Image;
use ratatui_image::protocol::Protocol;

use crate::app::{AppState, Logo, TrashStep};
use open_source_core::drives::{Drive, FsEntry};

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

pub fn setup_terminal() -> io::Result<Tui> {
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(io::stdout()))
}

pub fn restore_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

pub fn draw(frame: &mut Frame, state: &AppState, logo: &Logo, tree_icon: Option<&Protocol>) {
    match state {
        AppState::Onboarding {
            intro_frame,
            agreed,
            scroll,
            error,
        } => {
            if *intro_frame < 32 {
                draw_brand_intro(frame, logo, *intro_frame);
            } else {
                draw_terms(frame, *scroll, true, *agreed, error.as_deref());
            }
        }
        AppState::Loading {
            spinner_frame,
            started_at,
            scanned,
            total,
            cache_loading,
            progress_ratio,
            progress,
            ..
        } => draw_splash(
            frame,
            logo,
            *spinner_frame,
            started_at.elapsed(),
            *scanned,
            *total,
            *cache_loading,
            *progress_ratio,
            progress.as_ref(),
        ),
        AppState::Ready {
            drives,
            path,
            entries,
            selected,
            size_view,
            listing,
            stats_pending,
            stats_total,
            stats_done,
            terms_open,
            terms_scroll,
            pending_trash,
            action_notice,
            action_frame,
            error,
            ..
        } => {
            if *terms_open {
                draw_terms(frame, *terms_scroll, false, true, None);
            } else {
                draw_browser(
                    frame,
                    drives,
                    tree_icon,
                    path.as_deref(),
                    entries,
                    *selected,
                    *size_view,
                    *listing,
                    *stats_pending,
                    *stats_done,
                    *stats_total,
                    action_notice.as_deref(),
                    error.as_deref(),
                    pending_trash.as_ref(),
                    *action_frame,
                )
            }
        }
    }
}

const SPINNER_FRAMES: [char; 4] = ['|', '/', '-', '\\'];

fn draw_splash(
    frame: &mut Frame,
    logo: &Logo,
    spinner_frame: usize,
    elapsed: std::time::Duration,
    scanned: usize,
    total: usize,
    cache_loading: bool,
    progress_ratio: f64,
    progress: Option<&open_source_core::drives::ScanProgress>,
) {
    let area = frame.area();
    frame.render_widget(Block::default().borders(Borders::ALL), area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(logo.rows),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(area);

    let logo_lines = render_logo_lines(logo, 1.0);
    frame.render_widget(Paragraph::new(logo_lines), centered(logo.cols, sections[1]));
    frame.render_widget(
        Paragraph::new(Line::from(spectrum_spans(
            "◆ TREEMAP ◆",
            spinner_frame.wrapping_mul(3),
        )))
        .alignment(Alignment::Center),
        centered(24, sections[3]),
    );
    frame.render_widget(
        Paragraph::new(Line::from(spectrum_spans(
            "By Castron 2024",
            spinner_frame.wrapping_mul(3).wrapping_add(90),
        )))
        .style(Style::default().fg(Color::Gray).add_modifier(Modifier::DIM))
        .alignment(Alignment::Center),
        centered(24, sections[4]),
    );

    let spinner = SPINNER_FRAMES[spinner_frame % SPINNER_FRAMES.len()];
    let (ratio, status) = match (total, progress) {
        (0, _) => (
            0.0,
            format!(
                "{} | {}",
                if cache_loading {
                    "Opening local index"
                } else {
                    "Discovering mounted drives"
                },
                format_duration(elapsed)
            ),
        ),
        (_, Some(progress)) => {
            let ratio = ((scanned as f64 + progress_ratio) / total as f64).clamp(0.0, 1.0);
            let entries_per_second = progress.units_done as f64 / elapsed.as_secs_f64().max(1.0);
            (
                ratio,
                format!(
                    "Drive {}/{}: {} | {}/{} entries | {} files | {} dirs | {:.0}/s | {} | {}",
                    scanned + 1,
                    total,
                    progress.drive_name,
                    progress.units_done,
                    progress.units_total,
                    progress.files_scanned,
                    progress.dirs_scanned,
                    entries_per_second,
                    format_size(progress.bytes_found),
                    format_duration(elapsed),
                ),
            )
        }
        _ => (
            scanned as f64 / total as f64,
            format!(
                "Scanning drives ({scanned}/{total}) | {}",
                format_duration(elapsed)
            ),
        ),
    };

    let label = if total == 0 {
        format!("{spinner}")
    } else {
        format!("~{:.0}% {spinner}", ratio * 100.0)
    };
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL))
        .gauge_style(Style::default().fg(Color::Cyan))
        .ratio(ratio)
        .label(label);

    frame.render_widget(gauge, centered(68, sections[6]));
    frame.render_widget(
        Paragraph::new(status)
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::Gray)),
        centered(area.width.saturating_sub(4), sections[7]),
    );
}

fn draw_brand_intro(frame: &mut Frame, logo: &Logo, frame_number: usize) {
    let area = frame.area();
    frame.render_widget(Block::default().borders(Borders::ALL), area);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(logo.rows),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(area);

    let logo_opacity = (frame_number as f32 / 10.0).clamp(0.0, 1.0);
    let wordmark_opacity = ((frame_number.saturating_sub(10)) as f32 / 10.0).clamp(0.0, 1.0);
    frame.render_widget(
        Paragraph::new(render_logo_lines(logo, logo_opacity)),
        centered(logo.cols, sections[1]),
    );
    let title_color = Color::Rgb(
        (91.0 * wordmark_opacity) as u8,
        (210.0 * wordmark_opacity) as u8,
        (177.0 * wordmark_opacity) as u8,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "Tree",
                Style::default().fg(Color::Rgb(
                    (255.0 * wordmark_opacity) as u8,
                    (255.0 * wordmark_opacity) as u8,
                    (255.0 * wordmark_opacity) as u8,
                )),
            ),
            Span::styled(
                "Map",
                Style::default()
                    .fg(title_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .alignment(Alignment::Center),
        centered(24, sections[3]),
    );
}

fn draw_terms(frame: &mut Frame, scroll: u16, first_run: bool, agreed: bool, error: Option<&str>) {
    let area = frame.area();
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(area);
    let accent = Color::Rgb(91, 210, 177);
    let panel_bg = Color::Rgb(13, 20, 26);
    let panel_border = Color::Rgb(53, 72, 81);
    let header = Paragraph::new(vec![
        Line::from(Span::styled(
            "TREEMAP",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(
                "By Castron 2024",
                Style::default().fg(Color::Gray).add_modifier(Modifier::DIM),
            ),
            Span::styled("   /   TERMS & PRIVACY", Style::default().fg(accent)),
            Span::styled(
                error
                    .map(|message| format!("  |  {message}"))
                    .unwrap_or_default(),
                Style::default().fg(Color::Yellow),
            ),
        ]),
    ])
    .block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(accent))
            .style(Style::default().bg(Color::Rgb(10, 16, 21))),
    )
    .style(Style::default().bg(Color::Rgb(10, 16, 21)));
    frame.render_widget(header, sections[0]);

    let content = Paragraph::new(terms_lines())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(format!(
                    " TERMS & PRIVACY  /  {} ",
                    crate::consent::TERMS_VERSION
                ))
                .border_style(Style::default().fg(panel_border))
                .style(Style::default().bg(panel_bg)),
        )
        .scroll((scroll, 0))
        .wrap(Wrap { trim: true })
        .style(Style::default().fg(Color::Rgb(205, 216, 220)).bg(panel_bg));
    frame.render_widget(content, sections[1]);

    let footer_lines = if first_run {
        vec![
            Line::from(vec![
                Span::styled(
                    if agreed {
                        " SPACE  AGREED "
                    } else {
                        " SPACE  AGREE "
                    },
                    Style::default()
                        .fg(if agreed {
                            Color::Rgb(117, 239, 205)
                        } else {
                            Color::White
                        })
                        .bg(Color::Rgb(24, 38, 44))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("   "),
                keycap("Enter"),
                Span::raw(" Continue   "),
                keycap("↑/↓  j/k"),
                Span::raw(" Review   "),
                keycap("q / Esc"),
                Span::raw(" Exit"),
            ]),
            Line::from(Span::styled(
                "Read the sections above before agreeing.",
                Style::default().fg(Color::Rgb(141, 160, 168)),
            )),
        ]
    } else {
        vec![
            Line::from(vec![
                keycap("↑/↓  j/k"),
                Span::raw(" Scroll   "),
                keycap("PgUp/PgDn"),
                Span::raw(" Page   "),
                keycap("Home/End"),
                Span::raw(" Top/Bottom"),
            ]),
            Line::from(vec![
                keycap("Esc"),
                Span::raw(" Return   "),
                keycap("q"),
                Span::raw(" Quit"),
            ]),
        ]
    };
    frame.render_widget(
        Paragraph::new(footer_lines).style(
            Style::default()
                .fg(Color::Rgb(165, 181, 188))
                .bg(Color::Rgb(10, 16, 21)),
        ),
        sections[2],
    );
}

fn terms_lines() -> Vec<Line<'static>> {
    vec![
        section_heading("PURPOSE"),
        Line::from("TreeMap is a local filesystem inventory and navigation utility."),
        Line::from(""),
        section_heading("DATA & PRIVACY"),
        Line::from("TreeMap reads filesystem metadata: names, types, sizes, and timestamps."),
        Line::from(
            "This build does not read file contents, upload files, or send scan data to Castron. It includes no telemetry or network service.",
        ),
        Line::from(
            "The SQLite index is stored in DIRMAP_DATA_DIR when set; otherwise it uses your Windows user data directory.",
        ),
        Line::from(
            "The index is not encrypted. Anyone or any software with access to that account or folder may be able to read it.",
        ),
        Line::from(""),
        section_heading("ACCURACY & FILE SAFETY"),
        Line::from(
            "Last-access times can be disabled, delayed, or coarse. They are an imperfect signal, not proof a file is unused.",
        ),
        Line::from(
            "File actions show the item type, full path, and indexed size. Recycle Bin moves are generally restorable; permanent deletion bypasses it and may be unrecoverable.",
        ),
        Line::from(
            "Drive roots and common system/application locations are protected, but no path list is perfect. Links are not followed or offered for removal. Review every target and keep backups.",
        ),
        Line::from(""),
        section_heading("KEYBOARD GUIDE"),
        key_guide_row(
            "↑ / ↓   j / k",
            "Browser: move through rows. This page: scroll.",
        ),
        key_guide_row("ENTER   →", "Browser: open the selected drive or folder."),
        key_guide_row(
            "←   BACKSPACE   ESC",
            "Browser: go up one folder. Esc closes this page.",
        ),
        key_guide_row("r", "Browser: refresh the drive scan."),
        key_guide_row(
            "d",
            "Browser: open actions for the selected file or folder.",
        ),
        key_guide_row(
            "r / p",
            "In the file-action prompt: Recycle Bin / choose permanent deletion.",
        ),
        key_guide_row(
            "y   n / esc",
            "Confirm permanent deletion / cancel the prompt.",
        ),
        key_guide_row("b", "Toggle logical size-share bars and percentages."),
        key_guide_row(
            "t   q",
            "Browser: open Terms & Privacy / quit TreeMap. No mouse controls.",
        ),
        Line::from(""),
        section_heading("LIMITATION"),
        Line::from(
            "TreeMap is provided as-is. To the extent permitted by law, Castron is not responsible for loss arising from use of the app or actions you authorize. Nothing here excludes rights or liability that cannot legally be excluded.",
        ),
        Line::from(""),
        Line::from(
            "By selecting the checkbox and continuing, you acknowledge these terms for this version of TreeMap.",
        ),
    ]
}

fn section_heading(text: &'static str) -> Line<'static> {
    Line::from(Span::styled(
        text,
        Style::default()
            .fg(Color::Rgb(117, 239, 205))
            .add_modifier(Modifier::BOLD),
    ))
}

fn key_guide_row(keys: &'static str, description: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{keys:<22}"),
            Style::default()
                .fg(Color::Rgb(246, 192, 104))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(description, Style::default().fg(Color::Rgb(205, 216, 220))),
    ])
}

fn keycap(label: &'static str) -> Span<'static> {
    Span::styled(
        format!(" {label} "),
        Style::default()
            .fg(Color::Rgb(117, 239, 205))
            .bg(Color::Rgb(24, 38, 44))
            .add_modifier(Modifier::BOLD),
    )
}

/// Horizontally centers a fixed-width area within `area`.
fn centered(width: u16, area: Rect) -> Rect {
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(width.min(area.width)),
            Constraint::Min(0),
        ])
        .split(area);
    horizontal[1]
}

/// Renders the logo as one line per two source pixel rows, using the unicode
/// upper-half-block glyph (fg = top pixel, bg = bottom pixel). Pixels below an
/// alpha threshold leave that half of the style unset, so the terminal's own
/// background shows through instead of a solid box - true transparency rather
/// than compositing onto some fixed color.
fn render_logo_lines(logo: &Logo, opacity: f32) -> Vec<Line<'static>> {
    const ALPHA_THRESHOLD: u8 = 40;

    (0..logo.rows)
        .map(|row| {
            let spans: Vec<Span> = (0..logo.cols)
                .map(|col| {
                    let top = logo.image.get_pixel(col as u32, row as u32 * 2);
                    let bottom_y = row as u32 * 2 + 1;
                    let bottom = if bottom_y < logo.image.height() {
                        *logo.image.get_pixel(col as u32, bottom_y)
                    } else {
                        image::Rgba([0, 0, 0, 0])
                    };

                    let top_opacity = top[3] as f32 / 255.0 * opacity;
                    let bottom_opacity = bottom[3] as f32 / 255.0 * opacity;
                    let top_opaque = top_opacity * 255.0 > ALPHA_THRESHOLD as f32;
                    let bottom_opaque = bottom_opacity * 255.0 > ALPHA_THRESHOLD as f32;

                    let mut style = Style::default();
                    let ch = if top_opaque && bottom_opaque {
                        style = style
                            .fg(faded_rgb(top, opacity))
                            .bg(faded_rgb(&bottom, opacity));
                        "\u{2580}" // ▀ upper half block
                    } else if top_opaque {
                        style = style.fg(faded_rgb(top, opacity));
                        "\u{2580}"
                    } else if bottom_opaque {
                        style = style.fg(faded_rgb(&bottom, opacity));
                        "\u{2584}" // ▄ lower half block
                    } else {
                        " "
                    };
                    Span::styled(ch, style)
                })
                .collect();
            Line::from(spans)
        })
        .collect()
}

fn faded_rgb(pixel: &image::Rgba<u8>, opacity: f32) -> Color {
    Color::Rgb(
        (pixel[0] as f32 * opacity) as u8,
        (pixel[1] as f32 * opacity) as u8,
        (pixel[2] as f32 * opacity) as u8,
    )
}

fn draw_browser(
    frame: &mut Frame,
    drives: &[Drive],
    tree_icon: Option<&Protocol>,
    path: Option<&Path>,
    entries: &[FsEntry],
    selected: usize,
    size_view: bool,
    listing: bool,
    stats_pending: bool,
    stats_done: usize,
    stats_total: usize,
    action_notice: Option<&str>,
    error: Option<&str>,
    pending_trash: Option<&crate::app::PendingTrash>,
    action_frame: u64,
) {
    let area = frame.area();
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(tree_icon.map(|icon| icon.size().height).unwrap_or(3).max(3)),
        ])
        .split(area);
    let accent = Color::Rgb(91, 210, 177);
    let panel_bg = Color::Rgb(13, 20, 26);
    let panel_border = Color::Rgb(53, 72, 81);
    let brand_phase = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as usize
        / 100;

    let breadcrumb = path
        .map(|current| current.display().to_string())
        .unwrap_or_else(|| "Computer / Drives".to_string());
    let status = error
        .map(|message| format!("  |  {message}"))
        .or_else(|| action_notice.map(|message| format!("  |  {message}")))
        .or_else(|| listing.then(|| "  |  Reading folder...".to_string()))
        .or_else(|| stats_pending.then(|| format!("  |  Folder sizes {stats_done}/{stats_total}")))
        .unwrap_or_default();
    let context = if status.is_empty() {
        breadcrumb
    } else {
        format!("{status}    {breadcrumb}")
    };
    let header = Paragraph::new(vec![
        Line::from(spectrum_spans("TREEMAP", brand_phase)),
        Line::from(Span::styled(
            "By Castron 2024",
            Style::default().fg(Color::Gray).add_modifier(Modifier::DIM),
        )),
        Line::from(Span::styled(
            context,
            if status.is_empty() {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::Yellow)
            },
        )),
    ])
    .block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(accent))
            .style(Style::default().bg(Color::Rgb(10, 16, 21))),
    )
    .style(Style::default().bg(Color::Rgb(10, 16, 21)));
    frame.render_widget(header, sections[0]);

    draw_overview(frame, drives, path, entries, sections[1]);

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(sections[2]);
    let indexed_drive_total = drives
        .iter()
        .fold(0_u64, |total, drive| total.saturating_add(drive.total_size));
    let at_drive_root = path.is_some_and(|path| drives.iter().any(|drive| drive.root == path));
    let visible_total = entries
        .iter()
        .filter(|entry| !entry.is_link)
        .fold(0_u64, |total, entry| total.saturating_add(entry.size));
    let size_denominator = if path.is_none() {
        indexed_drive_total
    } else if at_drive_root {
        path.and_then(|path| {
            drives
                .iter()
                .find(|drive| drive.root == path)
                .map(|drive| drive.total_size)
        })
        .unwrap_or(visible_total)
    } else {
        visible_total
    };
    let percentage_ready = !stats_pending || path.is_none() || at_drive_root;
    let list_title = if size_view && path.is_none() {
        " SIZE SHARE | INDEXED VOLUMES "
    } else if size_view && at_drive_root {
        " SIZE SHARE | INDEXED DRIVE "
    } else if size_view {
        " SIZE SHARE | CURRENT FOLDER "
    } else if path.is_some() {
        " FOLDER CONTENTS | LARGEST FIRST "
    } else {
        " DRIVES "
    };

    let items = if path.is_some() {
        if entries.is_empty() {
            vec![ListItem::new(if listing {
                "Reading folder contents..."
            } else if error.is_some() {
                "Unable to read this folder"
            } else {
                "This folder is empty"
            })]
        } else {
            entries
                .iter()
                .map(|entry| {
                    if size_view {
                        let known_size = if entry.is_dir && !entry.is_link && entry.id.is_none() {
                            None
                        } else {
                            Some(entry.size)
                        };
                        size_share_item(
                            if entry.is_dir {
                                "▸ "
                            } else if entry.is_link {
                                "↗ "
                            } else {
                                "  "
                            },
                            &entry.name,
                            known_size,
                            size_denominator,
                            percentage_ready,
                        )
                    } else {
                        file_list_item(entry)
                    }
                })
                .collect()
        }
    } else if drives.is_empty() {
        vec![ListItem::new("No accessible drives found")]
    } else if size_view {
        let mut ordered_drives: Vec<_> = drives.iter().collect();
        ordered_drives.sort_unstable_by(|left, right| right.total_size.cmp(&left.total_size));
        ordered_drives
            .into_iter()
            .map(|drive| {
                size_share_item(
                    "▰ ",
                    &drive.name,
                    Some(drive.total_size),
                    size_denominator,
                    true,
                )
            })
            .collect()
    } else {
        drives.iter().map(drive_list_item).collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(list_title)
                .border_style(Style::default().fg(panel_border))
                .style(Style::default().bg(panel_bg)),
        )
        .highlight_symbol("› ")
        .style(Style::default().fg(Color::Rgb(205, 216, 220)).bg(panel_bg))
        .highlight_style(
            Style::default()
                .fg(Color::White)
                .bg(Color::Rgb(24, 57, 53))
                .add_modifier(Modifier::BOLD),
        );
    let mut list_state = ListState::default();
    let selectable_count = if path.is_some() {
        entries.len()
    } else {
        drives.len()
    };
    if selectable_count > 0 {
        list_state.select(Some(selected.min(selectable_count - 1)));
    }
    frame.render_stateful_widget(list, panes[0], &mut list_state);

    let detail_lines = if let Some(path) = path {
        directory_details(path, entries.get(selected))
    } else {
        drive_details(drives, selected, panes[1].height.saturating_sub(2) as usize)
    };
    let details = Paragraph::new(detail_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(" DETAILS ")
                .border_style(Style::default().fg(panel_border))
                .style(Style::default().bg(panel_bg)),
        )
        .wrap(Wrap { trim: true })
        .style(Style::default().fg(Color::Rgb(205, 216, 220)).bg(panel_bg));
    frame.render_widget(details, panes[1]);

    let key_style = Style::default()
        .fg(Color::Rgb(117, 239, 205))
        .bg(Color::Rgb(24, 38, 44))
        .add_modifier(Modifier::BOLD);
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(" ↑/↓ ", key_style),
        Span::raw(" Move   "),
        Span::styled("Enter", key_style),
        Span::raw(" Open   "),
        Span::styled("←", key_style),
        Span::raw(" Up   "),
        Span::styled("R", key_style),
        Span::raw(" Refresh   "),
        Span::styled("D", key_style),
        Span::raw(" Actions   "),
        Span::styled("B", key_style),
        Span::raw(" Size view   "),
        Span::styled("T", key_style),
        Span::raw(" Terms   "),
        Span::styled("Q", key_style),
        Span::raw(" Quit"),
    ]))
    .style(Style::default().fg(Color::Rgb(165, 181, 188)));
    let footer_columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(30.min(sections[2].width)),
        ])
        .split(sections[3]);
    frame.render_widget(footer, footer_columns[0]);
    if let Some(icon) = tree_icon {
        let brand = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(16), Constraint::Min(0)])
            .split(footer_columns[1]);
        frame.render_widget(Image::new(icon), brand[0]);
        frame.render_widget(
            Paragraph::new("TreeMap v1.0")
                .alignment(Alignment::Right)
                .style(Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            brand[1],
        );
    } else {
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                "TreeMap v1.0",
                Style::default()
                    .fg(Color::Rgb(91, 210, 177))
                    .add_modifier(Modifier::BOLD),
            )]))
            .alignment(Alignment::Right),
            footer_columns[1],
        );
    }
    if let Some(pending) = pending_trash {
        draw_trash_confirmation(frame, pending, action_frame);
    }
}

fn size_share_item(
    symbol: &str,
    name: &str,
    size: Option<u64>,
    denominator: u64,
    percentage_ready: bool,
) -> ListItem<'static> {
    const BAR_WIDTH: usize = 12;
    const NAME_WIDTH: usize = 18;

    let display_name = if name.chars().count() > NAME_WIDTH {
        format!("{}…", name.chars().take(NAME_WIDTH - 1).collect::<String>())
    } else {
        name.to_string()
    };
    let ratio = match (size, percentage_ready, denominator) {
        (Some(size), true, denominator) if denominator > 0 => {
            (size as f64 / denominator as f64).clamp(0.0, 1.0)
        }
        _ => 0.0,
    };
    let filled = ((ratio * BAR_WIDTH as f64).round() as usize).min(BAR_WIDTH);
    let share_color = if ratio >= 0.25 {
        Color::Rgb(91, 210, 177)
    } else if ratio >= 0.05 {
        Color::Rgb(246, 192, 104)
    } else {
        Color::Rgb(116, 144, 153)
    };
    let percent_label = if percentage_ready && size.is_some() {
        format!("{:>5.1}%", ratio * 100.0)
    } else {
        " --.-%".to_string()
    };
    let size_label = size.map(format_size).unwrap_or_else(|| "...".to_string());

    ListItem::new(Line::from(vec![
        Span::styled(
            symbol.to_string(),
            Style::default().fg(Color::Rgb(91, 210, 177)),
        ),
        Span::styled(
            format!("{display_name:<NAME_WIDTH$}"),
            Style::default().fg(Color::White),
        ),
        Span::raw(" "),
        Span::styled("█".repeat(filled), Style::default().fg(share_color)),
        Span::styled(
            "░".repeat(BAR_WIDTH - filled),
            Style::default().fg(Color::Rgb(53, 72, 81)),
        ),
        Span::raw(" "),
        Span::styled(percent_label, Style::default().fg(share_color)),
        Span::raw(" "),
        Span::styled(
            format!("{size_label:>9}"),
            Style::default().fg(Color::Rgb(205, 216, 220)),
        ),
    ]))
}

fn spectrum_spans(text: &str, phase: usize) -> Vec<Span<'static>> {
    text.chars()
        .enumerate()
        .map(|(index, character)| {
            Span::styled(
                character.to_string(),
                Style::default().fg(spectrum_color(phase + index * 18)),
            )
        })
        .collect()
}

fn spectrum_color(hue: usize) -> Color {
    let hue = (hue % 360) as f64;
    let chroma = 0.8075;
    let secondary = chroma * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let (red, green, blue) = match hue as u16 / 60 {
        0 => (chroma, secondary, 0.0),
        1 => (secondary, chroma, 0.0),
        2 => (0.0, chroma, secondary),
        3 => (0.0, secondary, chroma),
        4 => (secondary, 0.0, chroma),
        _ => (chroma, 0.0, secondary),
    };
    let lift = 0.1425;
    Color::Rgb(
        ((red + lift) * 255.0) as u8,
        ((green + lift) * 255.0) as u8,
        ((blue + lift) * 255.0) as u8,
    )
}

fn draw_overview(
    frame: &mut Frame,
    drives: &[Drive],
    path: Option<&Path>,
    entries: &[FsEntry],
    area: Rect,
) {
    let panel_bg = Color::Rgb(16, 24, 30);
    let panel_border = Color::Rgb(48, 65, 74);
    let label_style = Style::default().fg(Color::DarkGray);
    let value_style = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    let metrics = if path.is_none() {
        let logical_bytes: u64 = drives.iter().map(|drive| drive.total_size).sum();
        let capacities: Vec<_> = drives
            .iter()
            .filter_map(|drive| Some((drive.volume_total_bytes?, drive.volume_free_bytes?)))
            .collect();
        let physical_total: u64 = capacities.iter().map(|(total, _)| total).sum();
        let physical_free: u64 = capacities.iter().map(|(_, free)| free).sum();
        let used_percent = if physical_total == 0 {
            "n/a".to_string()
        } else {
            format!(
                "{:.1}%",
                (physical_total.saturating_sub(physical_free)) as f64 / physical_total as f64
                    * 100.0
            )
        };
        [
            ("VOLUMES", drives.len().to_string()),
            ("INDEXED LOGICAL", format_size(logical_bytes)),
            ("PHYSICAL USED", used_percent),
            (
                "SPACE AVAILABLE",
                if capacities.is_empty() {
                    "n/a".to_string()
                } else {
                    format_size(physical_free)
                },
            ),
        ]
    } else {
        [
            ("LOCATION", path.unwrap().display().to_string()),
            ("VISIBLE ITEMS", entries.len().to_string()),
            ("ORDER", "Largest first".to_string()),
            ("ACCESS SIGNAL", "Filesystem atime".to_string()),
        ]
    };
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(area);

    for (column, (label, value)) in columns.into_iter().zip(metrics) {
        let metric = Paragraph::new(vec![
            Line::from(Span::styled(label, label_style)),
            Line::from(Span::styled(value, value_style)),
        ])
        .block(
            Block::default()
                .borders(Borders::BOTTOM | Borders::RIGHT)
                .border_style(Style::default().fg(panel_border))
                .style(Style::default().bg(panel_bg)),
        )
        .style(Style::default().bg(panel_bg));
        frame.render_widget(metric, *column);
    }
}

fn draw_trash_confirmation(
    frame: &mut Frame,
    pending: &crate::app::PendingTrash,
    action_frame: u64,
) {
    let screen = frame.area();
    let width = screen.width.saturating_sub(4).min(86);
    let desired_height = match pending.step {
        TrashStep::Choose => 19,
        TrashStep::ConfirmPermanent => 18,
        TrashStep::MovingToRecycleBin | TrashStep::DeletingPermanently => 16,
    };
    let height = screen.height.saturating_sub(2).min(desired_height);
    let area = Rect {
        x: screen.x + screen.width.saturating_sub(width) / 2,
        y: screen.y + screen.height.saturating_sub(height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, area);

    let is_permanent = matches!(
        pending.step,
        TrashStep::ConfirmPermanent | TrashStep::DeletingPermanently
    );
    let accent = if is_permanent {
        Color::Rgb(242, 100, 104)
    } else {
        Color::Rgb(82, 211, 177)
    };
    let panel = Color::Rgb(15, 23, 30);
    let interior = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(match pending.step {
            TrashStep::Choose => " FILE ACTION  /  REVIEW ",
            TrashStep::ConfirmPermanent => " FINAL CHECK  /  PERMANENT ",
            TrashStep::MovingToRecycleBin => " FILE ACTION  /  MOVING ",
            TrashStep::DeletingPermanently => " FILE ACTION  /  DELETING ",
        })
        .border_style(Style::default().fg(accent))
        .style(Style::default().bg(panel));
    let inner = interior.inner(area);
    frame.render_widget(interior, area);

    let header = Paragraph::new(vec![
        Line::from(Span::styled(
            match pending.step {
                TrashStep::Choose => "Choose how to remove this item",
                TrashStep::ConfirmPermanent => "Permanent deletion cannot be undone",
                TrashStep::MovingToRecycleBin => "Moving item to the Recycle Bin",
                TrashStep::DeletingPermanently => "Permanently deleting item",
            },
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            match pending.step {
                TrashStep::Choose => "Review the target below. Nothing happens until you choose.",
                TrashStep::ConfirmPermanent => {
                    "This bypasses the Recycle Bin. Recovery may be impossible."
                }
                TrashStep::MovingToRecycleBin => "Windows is processing the selected item.",
                TrashStep::DeletingPermanently => "Do not close TreeMap while removal finishes.",
            },
            Style::default().fg(Color::Rgb(153, 171, 180)),
        )),
    ]);

    let target = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                if pending.is_dir { "FOLDER" } else { "FILE" },
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(
                pending.name.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(
            pending.target_path.display().to_string(),
            Style::default().fg(Color::Rgb(165, 180, 188)),
        )),
        Line::from(format!(
            "Indexed size   {}   /   {} bytes",
            format_size(pending.size),
            pending.size
        )),
    ])
    .block(
        Block::default()
            .borders(Borders::LEFT | Borders::TOP | Borders::BOTTOM)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(accent))
            .style(Style::default().bg(Color::Rgb(21, 31, 39))),
    )
    .wrap(Wrap { trim: true });

    let sections = match pending.step {
        TrashStep::Choose => Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(5),
                Constraint::Min(5),
                Constraint::Length(1),
            ])
            .split(inner),
        TrashStep::ConfirmPermanent => Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(5),
                Constraint::Length(2),
                Constraint::Min(4),
                Constraint::Length(1),
            ])
            .split(inner),
        TrashStep::MovingToRecycleBin | TrashStep::DeletingPermanently => Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(5),
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Min(0),
            ])
            .split(inner),
    };
    frame.render_widget(header, sections[0]);
    frame.render_widget(target, sections[1]);

    match pending.step {
        TrashStep::Choose => {
            let actions = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(sections[2]);
            render_action_card(
                frame,
                actions[0],
                "R   RECYCLE BIN",
                "Restorable from Windows",
                Color::Rgb(82, 211, 177),
                Color::Rgb(18, 39, 38),
            );
            render_action_card(
                frame,
                actions[1],
                "P   PERMANENT DELETE",
                "Bypasses the Recycle Bin",
                Color::Rgb(242, 100, 104),
                Color::Rgb(42, 27, 31),
            );
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("N / Esc", Style::default().fg(Color::White)),
                    Span::raw("  Cancel"),
                ]))
                .alignment(Alignment::Right)
                .style(Style::default().fg(Color::Rgb(153, 171, 180))),
                sections[3],
            );
        }
        TrashStep::ConfirmPermanent => {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "WARNING   This skips the Recycle Bin and may be unrecoverable.",
                    Style::default()
                        .fg(Color::Rgb(255, 137, 128))
                        .add_modifier(Modifier::BOLD),
                )),
                sections[2],
            );
            let actions = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
                .split(sections[3]);
            render_action_card(
                frame,
                actions[0],
                "Y   DELETE PERMANENTLY",
                "Confirm irreversible removal",
                Color::Rgb(242, 100, 104),
                Color::Rgb(42, 27, 31),
            );
            render_action_card(
                frame,
                actions[1],
                "N   CANCEL",
                "Keep this item",
                Color::Rgb(153, 171, 180),
                Color::Rgb(26, 35, 42),
            );
            frame.render_widget(
                Paragraph::new("Choose Y to confirm or N / Esc to cancel")
                    .alignment(Alignment::Right)
                    .style(Style::default().fg(Color::Rgb(153, 171, 180))),
                sections[4],
            );
        }
        TrashStep::MovingToRecycleBin | TrashStep::DeletingPermanently => {
            let progress = if is_permanent {
                if action_frame % 8 < 4 { 0.8 } else { 0.35 }
            } else {
                (action_frame % 20) as f64 / 20.0
            };
            frame.render_widget(
                Paragraph::new(Span::styled(
                    ["|", "/", "-", "\\"][action_frame as usize % 4],
                    Style::default().fg(accent).add_modifier(Modifier::BOLD),
                )),
                sections[2],
            );
            frame.render_widget(
                Gauge::default()
                    .gauge_style(Style::default().fg(accent).bg(Color::Rgb(31, 43, 50)))
                    .ratio(progress)
                    .label(if is_permanent {
                        "Permanent removal"
                    } else {
                        "Recycle Bin transfer"
                    }),
                sections[3],
            );
        }
    }
}

fn render_action_card(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    detail: &str,
    accent: Color,
    background: Color,
) {
    let card = Paragraph::new(vec![
        Line::from(Span::styled(
            title,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            detail,
            Style::default().fg(Color::Rgb(190, 203, 208)),
        )),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(accent))
            .style(Style::default().bg(background)),
    )
    .wrap(Wrap { trim: true });
    frame.render_widget(card, area);
}

fn drive_list_item(drive: &Drive) -> ListItem<'static> {
    let mut lines = vec![Line::from(vec![
        Span::styled("▰  ", Style::default().fg(Color::Rgb(91, 210, 177))),
        Span::styled(
            format!("{:<10}", drive.name),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("Indexed {:>10}", format_size(drive.total_size)),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled("   Enter to browse", Style::default().fg(Color::DarkGray)),
    ])];

    if let (Some(total), Some(free)) = (drive.volume_total_bytes, drive.volume_free_bytes) {
        let used = total.saturating_sub(free);
        let ratio = if total == 0 {
            0.0
        } else {
            (used as f64 / total as f64).clamp(0.0, 1.0)
        };
        let bar_width = 18usize;
        let filled = ((ratio * bar_width as f64).round() as usize).min(bar_width);
        let occupancy_color = if ratio >= 0.95 {
            Color::Red
        } else if ratio >= 0.8 {
            Color::Yellow
        } else {
            Color::Rgb(91, 210, 177)
        };
        lines.push(Line::from(vec![
            Span::raw("   "),
            Span::styled("█".repeat(filled), Style::default().fg(occupancy_color)),
            Span::styled(
                "░".repeat(bar_width - filled),
                Style::default().fg(Color::Rgb(58, 79, 77)),
            ),
            Span::styled(
                format!(
                    "  {:>5.1}%  {} used / {} free of {}",
                    ratio * 100.0,
                    format_size(used),
                    format_size(free),
                    format_size(total)
                ),
                Style::default().fg(Color::Rgb(246, 192, 104)),
            ),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "   Physical capacity unavailable",
            Style::default().fg(Color::DarkGray),
        )));
    }

    ListItem::new(lines)
}

fn file_list_item(entry: &FsEntry) -> ListItem<'static> {
    let (symbol, symbol_style) = if entry.is_link {
        ("↗ ", Style::default().fg(Color::DarkGray))
    } else if entry.is_dir {
        ("▸ ", Style::default().fg(Color::Rgb(91, 210, 177)))
    } else {
        ("  ", Style::default().fg(Color::White))
    };
    let name = if entry.name.chars().count() > 44 {
        format!("{}…", entry.name.chars().take(43).collect::<String>())
    } else {
        entry.name.clone()
    };
    let size = if entry.is_dir && entry.id.is_none() {
        "...".to_string()
    } else {
        format_size(entry.size)
    };
    let modified = entry
        .modified
        .map(format_age)
        .unwrap_or_else(|| "n/a".to_string());
    let accessed = entry
        .accessed
        .map(format_age)
        .unwrap_or_else(|| "n/a".to_string());

    ListItem::new(vec![
        Line::from(vec![
            Span::styled(symbol, symbol_style),
            Span::styled(name, Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled(
                format!("{:>9}", size),
                Style::default().fg(Color::Rgb(246, 192, 104)),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  Last Modified: ",
                Style::default().fg(Color::Rgb(141, 160, 168)),
            ),
            Span::styled(modified, Style::default().fg(Color::DarkGray)),
            Span::styled(
                "   Last Accessed: ",
                Style::default().fg(Color::Rgb(141, 160, 168)),
            ),
            Span::styled(accessed, Style::default().fg(Color::DarkGray)),
        ]),
    ])
}

fn drive_details(drives: &[Drive], selected: usize, available_rows: usize) -> Vec<Line<'static>> {
    let Some(drive) = drives.get(selected) else {
        return vec![Line::from("Select a drive to inspect")];
    };
    let mut lines = vec![
        Line::from(Span::styled(
            drive.name.clone(),
            Style::default()
                .fg(Color::Rgb(91, 210, 177))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "PHYSICAL UPTAKE",
            Style::default().fg(Color::Rgb(246, 192, 104)),
        )),
    ];
    if let (Some(total), Some(free)) = (drive.volume_total_bytes, drive.volume_free_bytes) {
        let used = total.saturating_sub(free);
        let ratio = if total == 0 {
            0.0
        } else {
            (used as f64 / total as f64).clamp(0.0, 1.0)
        };
        let bar_width = 14usize;
        let filled = ((ratio * bar_width as f64).round() as usize).min(bar_width);
        let color = if ratio >= 0.95 {
            Color::Red
        } else if ratio >= 0.8 {
            Color::Yellow
        } else {
            Color::Rgb(91, 210, 177)
        };
        lines.push(Line::from(vec![
            Span::styled("█".repeat(filled), Style::default().fg(color)),
            Span::styled(
                "░".repeat(bar_width - filled),
                Style::default().fg(Color::Rgb(58, 79, 77)),
            ),
            Span::styled(
                format!("  {:.1}%", ratio * 100.0),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(format!(
            "{} used / {} total",
            format_size(used),
            format_size(total)
        )));
        lines.push(Line::from(format!("{} bytes free", free)));
    } else {
        lines.push(Line::from("Physical capacity unavailable"));
    }
    lines.extend([
        Line::from(""),
        Line::from(Span::styled(
            "INDEXED CONTENT",
            Style::default().fg(Color::Rgb(246, 192, 104)),
        )),
        Line::from(format!(
            "{} logical / {} bytes",
            format_size(drive.total_size),
            drive.total_size
        )),
        Line::from(format!("Snapshot {}", format_age(drive.scanned_at))),
        Line::from(""),
        Line::from(Span::styled(
            "RECENT FILES",
            Style::default().fg(Color::Rgb(246, 192, 104)),
        )),
    ]);
    if drive.recent_files.is_empty() {
        lines.push(Line::from("No access times reported"));
    } else {
        let recent_limit = available_rows.saturating_sub(lines.len()).min(8);
        lines.extend(drive.recent_files.iter().take(recent_limit).map(|file| {
            Line::from(format!(
                "{}  {}  {}",
                format_age(file.accessed),
                format_size(file.size),
                file.path.file_name().unwrap_or_default().to_string_lossy()
            ))
        }));
    }
    lines.push(Line::from(""));
    lines.push(Line::from("Enter to browse"));
    lines
}

fn directory_details(path: &Path, selected: Option<&FsEntry>) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(
            "LOCATION",
            Style::default().fg(Color::Rgb(246, 192, 104)),
        )),
        Line::from(path.display().to_string()),
    ];
    if let Some(entry) = selected {
        let kind = if entry.is_link {
            "Link (not followed)"
        } else if entry.is_dir {
            "Folder"
        } else {
            "File"
        };
        let cached_summary = entry.id.is_some();
        let modified_label = if entry.is_dir && cached_summary {
            "Newest descendant modified"
        } else if entry.is_dir {
            "Folder modified"
        } else {
            "Last modified"
        };
        let accessed_label = if entry.is_dir && cached_summary {
            "Newest descendant atime"
        } else if entry.is_dir {
            "Folder atime"
        } else {
            "Last accessed (atime)"
        };
        let logical_size = if entry.is_dir && !cached_summary {
            "preparing from local index".to_string()
        } else {
            format!("{} ({} bytes)", format_size(entry.size), entry.size)
        };
        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                "SELECTED",
                Style::default().fg(Color::Rgb(246, 192, 104)),
            )),
            Line::from(Span::styled(
                entry.name.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(kind),
            Line::from(format!("Logical size: {logical_size}")),
            Line::from(format!(
                "{modified_label}: {}",
                entry
                    .modified
                    .map(format_age)
                    .unwrap_or_else(|| "n/a".to_string())
            )),
            Line::from(format!(
                "{accessed_label}: {}",
                entry
                    .accessed
                    .map(format_age)
                    .unwrap_or_else(|| "n/a".to_string())
            )),
        ]);
        if entry.is_dir {
            if cached_summary {
                lines.push(Line::from(format!(
                    "{} files, {} subfolders",
                    entry.file_count, entry.folder_count
                )));
            } else {
                lines.push(Line::from(
                    "Folder size and counts are being prepared from the local index...",
                ));
            }
        }
        let stale = entry
            .accessed
            .and_then(|accessed| std::time::SystemTime::now().duration_since(accessed).ok())
            .is_some_and(|age| age.as_secs() > 180 * 24 * 60 * 60);
        if entry.size >= 1024 * 1024 * 1024 && stale {
            lines.extend([
                Line::from(""),
                Line::from(Span::styled(
                    "Review: large and not recently accessed",
                    Style::default().fg(Color::Yellow),
                )),
                Line::from("Atime may be disabled or delayed; not a delete recommendation"),
            ]);
        }
    }
    lines
}

fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

fn format_age(accessed: std::time::SystemTime) -> String {
    let Ok(elapsed) = std::time::SystemTime::now().duration_since(accessed) else {
        return "clock skew".to_string();
    };

    let seconds = elapsed.as_secs();
    if seconds < 60 {
        format!("{seconds}s ago")
    } else if seconds < 3_600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3_600)
    } else if seconds < 2_592_000 {
        format!("{}d ago", seconds / 86_400)
    } else if seconds < 31_536_000 {
        format!("{}mo ago", seconds / 2_592_000)
    } else {
        format!("{}y ago", seconds / 31_536_000)
    }
}

fn format_duration(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    }
}
