//! Stage 3 & 6 shell interaction.
//!
//! Responsibilities:
//! - dock icon clicks launch applications (with fallbacks)
//! - launcher app discovery and search
//! - top-bar clock and status indicators (UTC for now)
//! - running-app indicators in dock via XDG app-ids

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use smithay::desktop::Window;

use crate::state::MitosGuiState;

// ============================================================================
// APPLICATION LAUNCHER
// ============================================================================

/// Launch an application by its dock ID, trying common fallback binaries.
pub fn launch_app(state: &mut MitosGuiState, id: &str) {
    let result = match id {
        "launcher" => {
            state.shell.toggle_launcher();
            return;
        }
        "files" => try_launch(&[
            "mitos-file-manager",
            "nautilus",
            "thunar",
            "pcmanfm",
            "dolphin",
        ]),
        "terminal" => try_launch(&[
            "mitos-terminal",
            "foot",
            "alacritty",
            "kitty",
            "weston-terminal",
            "xterm",
        ]),
        "browser" => try_launch_browser(),
        "settings" => try_launch(&[
            "mitos-settings",
            "gnome-control-center",
            "xfce4-settings-manager",
            "cinnamon-settings",
        ]),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Unknown app: {id}"),
        )),
    };

    match result {
        Ok(()) => tracing::info!("MITOS GUI: launched {id}"),
        Err(err) => tracing::warn!("MITOS GUI: failed to launch {id}: {err}"),
    }
}

/// Attempt to spawn one of the provided binaries in order.
fn try_launch(binaries: &[&str]) -> std::io::Result<()> {
    for bin in binaries {
        match Command::new(bin).spawn() {
            Ok(_) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "None of the fallback binaries could be launched",
    ))
}

/// Attempt to spawn a web browser, falling back to xdg-open.
fn try_launch_browser() -> std::io::Result<()> {
    let browsers = ["firefox", "chromium-browser", "google-chrome", "epiphany"];
    for bin in browsers {
        match Command::new(bin).spawn() {
            Ok(_) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        }
    }
    Command::new("xdg-open").arg("about:blank").spawn().map(|_| ())
}

// ============================================================================
// RUNNING APP TRACKING
// ============================================================================

/// Mark a dock item as running based on XDG toplevel app-ids.
pub fn update_running_state(state: &mut MitosGuiState) {
    let mut running_dock_ids: Vec<&'static str> = Vec::new();
    let mut active_dock_id: Option<&'static str> = None;

    let focused_app_id = state
        .focused_window
        .as_ref()
        .and_then(|w| app_id_for_window(w));

    if let Some(ref id) = focused_app_id {
        active_dock_id = dock_id_for_app_id(id);
    }

    // Scan all mapped windows
    for window in state.space.elements() {
        if let Some(app_id) = app_id_for_window(window) {
            if let Some(dock_id) = dock_id_for_app_id(&app_id) {
                if !running_dock_ids.contains(&dock_id) {
                    running_dock_ids.push(dock_id);
                }
            }
        }
    }

    // Update dock layout
    for item in &mut state.shell.dock_layout.items {
        item.running = running_dock_ids.contains(&item.id);
        item.active = active_dock_id == Some(item.id);
    }

    state.pending_full_redraw = true;
}

/// Extract the XDG app-id from a window's toplevel surface.
fn app_id_for_window(window: &Window) -> Option<String> {
    window
        .toplevel()
        .and_then(|t| t.current_state().app_id.clone())
}

/// Map an XDG app-id to a MITOS dock ID.
fn dock_id_for_app_id(app_id: &str) -> Option<&'static str> {
    let a = app_id.to_ascii_lowercase();
    if a.contains("file") || a.contains("nautilus") || a.contains("thunar") || a.contains("dolphin") {
        Some("files")
    } else if a.contains("terminal") || a.contains("foot") || a.contains("alacritty") || a.contains("kitty") {
        Some("terminal")
    } else if a.contains("firefox") || a.contains("chrom") || a.contains("epiphany") {
        Some("browser")
    } else if a.contains("settings") || a.contains("control") {
        Some("settings")
    } else {
        None
    }
}

// ============================================================================
// TOP BAR CLOCK
// ============================================================================

/// Get current time as a formatted string.
/// Note: Uses UTC until a timezone library (e.g. `chrono` or `time`) is added.
pub fn current_time_string() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    let secs = now.as_secs();
    let hours = ((secs / 3600) % 24) as u32;
    let minutes = ((secs / 60) % 60) as u32;

    format!("{hours:02}:{minutes:02}")
}

/// Convert days since Unix epoch (1970-01-01) to (year, month, day).
/// Based on Howard Hinnant's civil_from_days algorithm.
fn civil_from_days(days: i32) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // year of era [0, 399]
    let y = (yoe as i32) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // month [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // day [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // month [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Get current date as a formatted string.
/// Note: Uses UTC until a timezone library is added.
pub fn current_date_string() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    let secs = now.as_secs() as i64;
    let days = (secs / 86400) as i32;
    
    let (year, month, day) = civil_from_days(days);

    format!("{year:04}-{month:02}-{day:02}")
}

// ============================================================================
// LAUNCHER APP DISCOVERY
// ============================================================================

/// A discovered application that can be launched.
#[derive(Clone, Debug)]
pub struct AppEntry {
    pub name: String,
    pub exec: String,
    pub icon: String,
    pub categories: Vec<String>,
}

/// Scan common XDG application directories for .desktop files.
pub fn discover_apps() -> Vec<AppEntry> {
    let mut all_paths = Vec::new();

    if let Ok(home) = std::env::var("HOME") {
        let user_apps = format!("{home}/.local/share/applications");
        if std::path::Path::new(&user_apps).exists() {
            all_paths.push(user_apps);
        }
    }

    all_paths.push("/usr/share/applications".to_string());
    all_paths.push("/usr/local/share/applications".to_string());

    discover_apps_in_dirs(&all_paths)
}

fn discover_apps_in_dirs(dirs: &[String]) -> Vec<AppEntry> {
    let mut apps = Vec::new();

    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("desktop") {
                if let Some(app) = parse_desktop_file(&path) {
                    apps.push(app);
                }
            }
        }
    }

    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps
}

fn parse_desktop_file(path: &std::path::Path) -> Option<AppEntry> {
    let content = std::fs::read_to_string(path).ok()?;

    let mut name = String::new();
    let mut exec = String::new();
    let mut icon = String::new();
    let mut categories = Vec::new();
    let mut no_display = false;

    for line in content.lines() {
        let line = line.trim();

        if line.starts_with("Name=") {
            name = line.strip_prefix("Name=").unwrap_or_default().to_string();
        } else if line.starts_with("Exec=") {
            exec = line.strip_prefix("Exec=").unwrap_or_default().to_string();
            // Strip standard field codes like %f, %u, %i, %c, %k
            exec = exec
                .replace("%f", "")
                .replace("%F", "")
                .replace("%u", "")
                .replace("%U", "")
                .replace("%i", "")
                .replace("%c", "")
                .replace("%k", "")
                .trim()
                .to_string();
        } else if line.starts_with("Icon=") {
            icon = line.strip_prefix("Icon=").unwrap_or_default().to_string();
        } else if line.starts_with("Categories=") {
            let cats = line.strip_prefix("Categories=").unwrap_or_default();
            categories = cats.split(';').map(|s| s.to_string()).collect();
        } else if line == "NoDisplay=true" {
            no_display = true;
        }
    }

    if name.is_empty() || exec.is_empty() || no_display {
        return None;
    }

    Some(AppEntry {
        name,
        exec,
        icon,
        categories,
    })
}

/// Search discovered apps by name or category.
pub fn search_apps(apps: &[AppEntry], query: &str) -> Vec<AppEntry> {
    if query.is_empty() {
        return apps.to_vec();
    }

    let query_lower = query.to_lowercase();

    apps.iter()
        .filter(|app| {
            app.name.to_lowercase().contains(&query_lower)
                || app.categories.iter().any(|c| c.to_lowercase().contains(&query_lower))
        })
        .cloned()
        .collect()
}

// ============================================================================
// LAUNCHER INTERACTION
// ============================================================================

/// Launch an app from the launcher search results.
pub fn launch_app_entry(entry: &AppEntry) {
    let parts: Vec<&str> = entry.exec.split_whitespace().collect();
    if parts.is_empty() {
        return;
    }

    let result = Command::new(parts[0]).args(&parts[1..]).spawn();

    match result {
        Ok(_) => tracing::info!("MITOS GUI: launched {}", entry.name),
        Err(err) => tracing::warn!("MITOS GUI: failed to launch {}: {err}", entry.name),
    }
}
