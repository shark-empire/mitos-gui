//! Stage 7 (Phase 3): Desktop icons.
//!
//! Lists `~/Desktop` and lays it out as an auto-arranged icon grid --
//! no drag-to-reposition, same reasoning as every other click-only
//! interaction in this codebase (Quick Settings' sliders, the
//! workspace overview's lack of drag-between-cells): dragging needs
//! pointer-motion tracking across a multi-step gesture that nothing
//! here has wired up yet.
//!
//! Icons are colored swatches plus a label, same placeholder style as
//! the launcher/dock/switcher/overview -- MITOS doesn't parse real
//! desktop icon themes anywhere, this doesn't start now. Folders get a
//! small tab on the swatch so they read differently from files.
//!
//! Drawn on the primary output only: the layout is anchored to the
//! top bar/dock panels in `state.shell`, which describe that one
//! output, and pointer hit-testing works in global coordinates -- the
//! same single-output simplification the dock already makes.
//!
//! Opening a file shells out to `xdg-open`, the standard
//! "open with whatever's associated" command on Linux, same
//! best-effort fire-and-forget shape as `audio.rs`/`brightness.rs`.
//! Opening a folder tries MITOS's own file manager first, then common
//! fallbacks, mirroring `shell_interaction::launch_app`'s pattern.
//!
//! **Deliberately not implemented: deleting a file from here.** A
//! destructive filesystem operation that's never been exercised
//! against a real session is a bad place to find out the path
//! handling was subtly wrong. Opening and creating a new folder are
//! both easily undone by the user if something's off; deleting isn't,
//! so it stays out of this pass.

use std::path::PathBuf;
use std::process::Command;

#[derive(Clone)]
pub struct DesktopIcon {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

/// `~/Desktop`, or `None` if `$HOME` isn't set (never expected in a
/// real session, but this is a compositor -- it shouldn't panic over
/// an unusual environment).
fn desktop_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Desktop"))
}

/// Lists `~/Desktop`, sorted with folders first then alphabetically
/// within each group (the common convention). An empty list (desktop
/// dir missing, empty, or unreadable) is a normal, silent outcome --
/// not every account has a populated Desktop folder.
pub fn discover_icons() -> Vec<DesktopIcon> {
    let Some(dir) = desktop_dir() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };

    let mut icons: Vec<DesktopIcon> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            // Skip dotfiles -- same convention file managers use for
            // "hidden unless you ask to see them".
            if name.starts_with('.') {
                return None;
            }
            // `path.is_dir()` follows symlinks (a symlink to a folder
            // opens as a folder); `DirEntry::file_type()` wouldn't.
            let is_dir = path.is_dir();
            Some(DesktopIcon { name, path, is_dir })
        })
        .collect();

    icons.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    icons
}

const CELL_W: i32 = 92;
const CELL_H: i32 = 96;
pub const SWATCH: i32 = 52;
const TOP_MARGIN: i32 = 16;
const LEFT_MARGIN: i32 = 16;

/// Longest label shown under an icon before it's cut with "...": the
/// label is rasterized to a fixed-size texture, so a long filename
/// can't be clipped to the cell after the fact.
pub const MAX_LABEL_CHARS: usize = 12;

/// Top-left corner of icon `index`'s cell, laid out as a single
/// top-to-bottom column that wraps into a new column once it reaches
/// `max_rows` -- the traditional desktop-icon arrangement order.
pub fn icon_position(index: usize, max_rows: usize) -> (i32, i32) {
    let max_rows = max_rows.max(1);
    let col = index / max_rows;
    let row = index % max_rows;
    (
        LEFT_MARGIN + col as i32 * CELL_W,
        TOP_MARGIN + row as i32 * CELL_H,
    )
}

/// How many icon rows fit in a band of height `usable_h`.
pub fn max_rows_for_height(usable_h: i32) -> usize {
    ((usable_h - TOP_MARGIN) / CELL_H).max(1) as usize
}

pub fn cell_size() -> (i32, i32) {
    (CELL_W, CELL_H)
}

/// The vertical origin icons start from (the top bar's height, so
/// icons never sit underneath it) and how many rows fit before
/// reaching the dock -- derived from `state.shell` the same way for
/// both the renderer and pointer hit-testing, so a click always lands
/// on the icon it visually looks like it's over.
pub fn layout_origin_and_rows(state: &crate::state::MitosGuiState) -> (i32, usize) {
    let top_bar_h = state.shell.top_bar.map(|p| p.size.1).unwrap_or(0);
    let dock_top = state.shell.dock.map(|p| p.position.1).unwrap_or(i32::MAX);
    let usable_h = dock_top.saturating_sub(top_bar_h).max(CELL_H);
    (top_bar_h, max_rows_for_height(usable_h))
}

/// The label shown under an icon: `name`, cut to `MAX_LABEL_CHARS`
/// characters with a trailing "..." when it had to be shortened.
pub fn display_label(name: &str) -> String {
    if name.chars().count() <= MAX_LABEL_CHARS {
        return name.to_string();
    }
    let mut s: String = name.chars().take(MAX_LABEL_CHARS - 3).collect();
    s.push_str("...");
    s
}

/// Opens `icon` -- a folder through MITOS's file manager (or a
/// fallback), a file through `xdg-open`. Fire-and-forget, same as
/// `shell_interaction::launch_app`, including its failure mode: a
/// missing handler is logged and shown as a notification toast.
pub fn open_icon(state: &mut crate::state::MitosGuiState, icon: &DesktopIcon) {
    let path_str = icon.path.to_string_lossy().to_string();

    let result = if icon.is_dir {
        try_launch_with_arg(FILE_MANAGERS, &path_str)
    } else {
        try_launch_with_arg(&["xdg-open"], &path_str)
    };

    match result {
        Ok(()) => tracing::info!("MITOS GUI: opened {path_str}"),
        Err(err) => {
            tracing::warn!("MITOS GUI: failed to open {path_str}: {err}");
            state.notifications.push(
                "MITOS",
                "Couldn't open",
                &format!("No application available for {}", icon.name),
            );
        }
    }
}

const FILE_MANAGERS: &[&str] = &["mitos-file-manager", "nautilus", "thunar", "pcmanfm", "dolphin"];

const TERMINALS: &[&str] = &["mitos-terminal", "foot", "alacritty", "kitty", "weston-terminal", "xterm"];

fn try_launch_with_arg(binaries: &[&str], arg: &str) -> std::io::Result<()> {
    for bin in binaries {
        match Command::new(bin).arg(arg).spawn() {
            Ok(_) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no handler found"))
}

/// Opens the Desktop folder itself in the file manager (the context
/// menu's "Open Desktop in Files"). Creates `~/Desktop` first if it
/// doesn't exist yet -- harmless, and better than handing a file
/// manager a path that isn't there.
pub fn open_desktop_in_file_manager(state: &mut crate::state::MitosGuiState) {
    let Some(dir) = desktop_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let path_str = dir.to_string_lossy().to_string();

    if let Err(err) = try_launch_with_arg(FILE_MANAGERS, &path_str) {
        tracing::warn!("MITOS GUI: failed to open {path_str}: {err}");
        state.notifications.push("MITOS", "Couldn't open", "No file manager available");
    }
}

/// Opens a terminal whose working directory is `~/Desktop` (the
/// context menu's "Open Terminal Here" -- "here" being the desktop
/// that was right-clicked), falling back to `$HOME` if there's no
/// Desktop folder. Same terminal fallback order as `launch_app`.
pub fn open_terminal_here(state: &mut crate::state::MitosGuiState) {
    let dir = desktop_dir()
        .filter(|d| d.is_dir())
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from));

    for bin in TERMINALS {
        let mut cmd = Command::new(bin);
        if let Some(d) = &dir {
            cmd.current_dir(d);
        }
        match cmd.spawn() {
            Ok(_) => {
                tracing::info!("MITOS GUI: opened terminal in {dir:?}");
                return;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                tracing::warn!("MITOS GUI: failed to open terminal: {e}");
                break;
            }
        }
    }

    state.notifications.push("MITOS", "Couldn't open terminal", "No terminal application available");
}

/// Creates `~/Desktop/New Folder` (or `New Folder 2`, `3`, ... past
/// any existing one) for the context menu's "New Folder". Best-effort:
/// returns `None` on any failure, which just means no folder appeared.
/// Creating a folder has no failure mode that loses anything, unlike
/// delete, which is exactly why this exists in the context menu and
/// delete doesn't.
pub fn create_new_folder() -> Option<PathBuf> {
    let dir = desktop_dir()?;
    std::fs::create_dir_all(&dir).ok()?;

    let base = "New Folder";
    let mut candidate = dir.join(base);
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{base} {n}"));
        n += 1;
    }

    std::fs::create_dir(&candidate).ok()?;
    Some(candidate)
}
