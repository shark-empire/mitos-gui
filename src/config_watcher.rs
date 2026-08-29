//! Watches ~/.config/mitos/home.conf with inotify and notifies the
//! compositor through a calloop channel when it changes.
//!
//! MITOS Files writes this file when the user changes settings.
//! mitos-gui reloads it live, so the whole desktop stays in sync.

use calloop::channel::Sender;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct ConfigChanged;

pub struct ConfigWatcher {
    _watcher: RecommendedWatcher,
}

pub fn config_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("mitos").join("home.conf"));
        }
    }

    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".config").join("mitos").join("home.conf"))
}

impl ConfigWatcher {
    pub fn start(sender: Sender<ConfigChanged>) -> Option<Self> {
        let path = config_path()?;

        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let mut watcher = notify::recommended_watcher(
            move |res: Result<Event, notify::Error>| {
                let Ok(event) = res else { return };

                match event.kind {
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {
                        // Small debounce: fs::write can emit several events.
                        std::thread::sleep(Duration::from_millis(50));
                        let _ = sender.send(ConfigChanged);
                    }
                    _ => {}
                }
            },
        )
        .ok()?;

        // Watch the DIRECTORY, not the file.
        // fs::write() can replace the inode, which kills a file watch.
        let dir = path.parent()?;
        watcher.watch(dir, RecursiveMode::NonRecursive).ok()?;

        tracing::info!("MITOS GUI: watching {} for changes", dir.display());

        Some(Self { _watcher: watcher })
    }
}
