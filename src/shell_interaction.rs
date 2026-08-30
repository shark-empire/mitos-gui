//! Stage 3 & 6 shell interaction.
//!
//! Responsibilities:
//! - dock icon clicks launch applications
//! - launcher app discovery and search
//! - top-bar clock and status indicators
//! - running-app indicators in dock

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use smithay::desktop::Window;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::seat::WaylandFocus;

use crate::desktop::DockItem;
use crate::state::MitosGuiState;

// ============================================================================
// APPLICATION LAUNCHER
// ============================================================================

/// Launch an application by its dock ID.
pub fn launch_app(state: &mut MitosGuiState, id: &str) {
    let result = match id {
        "launcher" => {
            state.shell.toggle_launcher();
            Ok(())
        }
        "files" => Command::new("mitos-file-manager").spawn().map(|_| ()),
        "terminal" => Command::new("weston-terminal").spawn().map(|_| ()),
        "browser" => Command::new("xdg-open").arg("https://example.com").spawn().map(|_| ()),
        "settings" => Command::new("mitos-settings").spawn().map(|_| ()),
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

// ============================================================================
// RUNNING APP TRACKING
// ============================================================================

/// Mark a dock item as running based on window metadata.
pub fn update_running_state(state: &mut MitosGuiState) {
    let mut running_apps: HashMap<String, bool> = HashMap::new();

    // Scan all mapped windows
    for window in state.space.elements() {
        if let Some(surface) = window.wl_surface() {
            if let Some(app_id) = get_app_id(&surface) {
                running_apps.insert(app_id, true);
            }
        }
    }

    // Update dock layout
    for item in &mut state.shell.dock_layout.items {
        item.running = running_apps.contains_key(item.id);
        item.active = state
            .focused_window
            .as_ref()
            .and_then(|w| w.wl_surface())
            .and_then(|s| get_app_id(&s))
            .map(|id| id == item.id)
            .unwrap_or(false);
    }

    state.pending_full_redraw = true;
}

/// Extract app ID from a Wayland surface (client sets this).
fn get_app_id(surface: &WlSurface) -> Option<String> {
    surface
        .user_data()
        .get::<Mutex<String>>()
        .map(|m| m.lock().unwrap().clone())
}

// ============================================================================
// TOP BAR CLOCK
// ============================================================================

/// Get current time as a formatted string.
pub fn current_time_string() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    let secs = now.as_secs();
    let hours = ((secs / 3600) % 24) as u32;
    let minutes = ((secs / 60) % 60) as u32;

    format!("{hours:02}:{minutes:02}")
}

/// Get current date as a formatted string.
pub fn current_date_string() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    let secs = now.as_secs();
    let days = (secs / 86400) as u32;
    let year = 1970 + (days / 365);
    let day_of_year = days % 365;
    let month = (day_of_year / 30).min(11) + 1;
    let day = (day_of_year % 30) + 1;

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
    let mut apps = Vec::new();

    let search_paths = vec![
        "/usr/share/applications",
        "/usr/local/share/applications",
    ];

    if let Ok(home) = std::env::var("HOME") {
        let user_apps = format!("{home}/.local/share/applications");
        if std::path::Path::new(&user_apps).exists() {
            // Prepend user path so it's checked first
            let mut paths = vec![user_apps];
            paths.extend(search_paths);
            return discover_apps_in_dirs(&paths);
        }
    }

    discover_apps_in_dirs(&search_paths)
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
            // Strip field codes like %f, %u
            exec = exec.replace("%f", "").replace("%F", "")
                       .replace("%u", "").replace("%U", "")
                       .trim().to_string();
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

    let result = Command::new(parts[0])
        .args(&parts[1..])
        .spawn();

    match result {
        Ok(_) => tracing::info!("MITOS GUI: launched {}", entry.name),
        Err(err) => tracing::warn!("MITOS GUI: failed to launch {}: {err}", entry.name),
    }
}
