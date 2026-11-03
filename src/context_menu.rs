//! Stage 7 (Phase 3): a general-purpose right-click context menu.
//!
//! Right now the desktop background, desktop icons and the Bluetooth
//! pairing prompt open one, but the type itself (`ContextMenu` + `MenuAction`) doesn't assume
//! that -- a dock icon or window titlebar context menu later is just
//! another `MenuAction` variant and another constructor, not a new
//! menu system.

use smithay::utils::{Logical, Rectangle, Size};

use crate::state::MitosGuiState;

#[derive(Clone)]
pub enum MenuAction {
    /// Open the desktop icon at this index into `state.desktop_icons`.
    /// The index is only resolved at click time against whatever the
    /// list looks like then (it could in principle have been
    /// refreshed while the menu was open), and ignored if it no longer
    /// points at anything.
    OpenIcon(usize),
    NewFolder,
    OpenTerminalHere,
    OpenDesktopInFiles,
    RefreshDesktopIcons,
    /// Answer the Bluetooth pairing request waiting on screen (the prompt
    /// is a context menu -- see `services.rs`).
    BluetoothPair { accept: bool },
}

pub struct ContextMenu {
    pub x: i32,
    pub y: i32,
    pub items: Vec<(String, MenuAction)>,
}

const ITEM_H: i32 = 32;
const ITEM_W: i32 = 210;
const PAD: i32 = 6;

impl ContextMenu {
    pub fn for_icon(x: i32, y: i32, icon_name: &str, icon_index: usize) -> Self {
        Self {
            x,
            y,
            items: vec![(
                format!("Open {}", crate::desktop_icons::display_label(icon_name)),
                MenuAction::OpenIcon(icon_index),
            )],
        }
    }

    pub fn for_desktop(x: i32, y: i32) -> Self {
        Self {
            x,
            y,
            items: vec![
                ("New Folder".to_string(), MenuAction::NewFolder),
                ("Open Terminal Here".to_string(), MenuAction::OpenTerminalHere),
                ("Open Desktop in Files".to_string(), MenuAction::OpenDesktopInFiles),
                ("Refresh".to_string(), MenuAction::RefreshDesktopIcons),
            ],
        }
    }

    /// The menu's full bounding box, clamped so it doesn't open partly
    /// off-screen on the right/bottom edge (`x`/`y` are where the
    /// click happened, not necessarily where the menu's own top-left
    /// corner should land).
    pub fn rect(&self, output_size: Size<i32, Logical>) -> Rectangle<i32, Logical> {
        let h = PAD * 2 + self.items.len() as i32 * ITEM_H;
        let w = ITEM_W;

        let x = self.x.min((output_size.w - w).max(0));
        let y = self.y.min((output_size.h - h).max(0));

        Rectangle::new((x, y).into(), (w, h).into())
    }

    /// Each item's row rectangle, in the same order as `self.items`,
    /// positioned relative to `self.rect()`'s (possibly clamped)
    /// origin -- shared by the renderer and pointer hit-testing for
    /// the same reason every other `compute_geometry`-style function
    /// in this codebase is.
    pub fn item_rects(&self, output_size: Size<i32, Logical>) -> Vec<Rectangle<i32, Logical>> {
        let base = self.rect(output_size);
        (0..self.items.len())
            .map(|i| {
                Rectangle::new(
                    (base.loc.x + PAD, base.loc.y + PAD + i as i32 * ITEM_H).into(),
                    (base.size.w - PAD * 2, ITEM_H).into(),
                )
            })
            .collect()
    }
}

/// Carries out a clicked menu item. The menu itself has already been
/// closed by the caller (`pointer.rs`) by the time this runs.
pub fn execute(state: &mut MitosGuiState, action: MenuAction) {
    match action {
        MenuAction::OpenIcon(i) => {
            if let Some(icon) = state.desktop_icons.get(i).cloned() {
                crate::desktop_icons::open_icon(state, &icon);
            }
        }
        MenuAction::NewFolder => {
            match crate::desktop_icons::create_new_folder() {
                Some(path) => tracing::info!("MITOS GUI: created {}", path.display()),
                None => state.notifications.push("MITOS", "Couldn't create folder", "~/Desktop isn't writable"),
            }
            state.desktop_icons = crate::desktop_icons::discover_icons();
            state.desktop_icon_selected = None;
        }
        MenuAction::OpenTerminalHere => crate::desktop_icons::open_terminal_here(state),
        MenuAction::OpenDesktopInFiles => crate::desktop_icons::open_desktop_in_file_manager(state),
        MenuAction::RefreshDesktopIcons => {
            state.desktop_icons = crate::desktop_icons::discover_icons();
            state.desktop_icon_selected = None;
        }
        MenuAction::BluetoothPair { accept } => state.respond_bluetooth_pairing(accept),
    }
}
