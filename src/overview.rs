//! Stage 7 (Phase 3): Workspace overview.
//!
//! A zoomed-out view of every workspace at once (Super+O): a 2x2 grid,
//! one cell per workspace, each listing the windows on it. Pick a cell
//! -- click it, arrow to it and press Enter -- to jump there.
//!
//! Windows are shown as a colored swatch plus title rather than as
//! live scaled-down thumbnails: real thumbnails would mean rendering
//! every window's surface tree into an offscreen texture and scaling
//! it per frame, a much larger renderer undertaking than the rest of
//! this overview. The swatch color comes from `renderer::
//! icon_swatch_color` keyed on the title, so a window has the same
//! color here as in the task switcher and launcher.
//!
//! Not built: dragging a window from one cell to another to move it
//! between workspaces. Super+Shift+1..4 already does that from the
//! keyboard; drag would need pointer-motion tracking across a
//! multi-event gesture, which nothing else clickable in this codebase
//! has either (the Quick Settings sliders are click-to-set for the
//! same reason).

use smithay::utils::{Logical, Rectangle, Size};

use crate::state::{MitosGuiState, WORKSPACE_COUNT};

// The 2x2 grid in `compute_geometry`, and `handle_overview_input`'s
// +-2 step for up/down, both assume exactly four workspaces. Fail the
// build loudly if `state::WORKSPACE_COUNT` ever changes, rather than
// quietly drawing a wrong layout.
const _: () = assert!(
    WORKSPACE_COUNT == 4,
    "overview.rs lays workspaces out as a 2x2 grid; revisit compute_geometry and handle_overview_input if WORKSPACE_COUNT changes"
);

/// Height reserved at the top of each cell for its header line.
pub const HEADER_H: i32 = 34;
/// Height of one window row inside a cell.
pub const ROW_H: i32 = 26;
/// Inner padding inside a cell.
pub const PAD: i32 = 12;

const GAP: i32 = 20;
const MAX_TITLE_CHARS: usize = 30;

/// Panel and per-cell geometry for the given output size. Cells are
/// indexed by workspace (0 = top-left, 1 = top-right, 2 = bottom-left,
/// 3 = bottom-right), so arrow-key movement is +-1 for left/right and
/// +-2 for up/down. Shared by the renderer (to draw) and pointer
/// hit-testing (to know which cell was clicked), for the same reason
/// as `osk::compute_geometry` and `quick_settings::compute_geometry`.
pub fn compute_geometry(
    output_size: Size<i32, Logical>,
) -> (Rectangle<i32, Logical>, Vec<Rectangle<i32, Logical>>) {
    let panel_w = (output_size.w - 120).clamp(320, 1100);
    let panel_h = (output_size.h - 160).clamp(240, 640);
    let panel_x = (output_size.w - panel_w) / 2;
    let panel_y = (output_size.h - panel_h) / 2;

    let panel = Rectangle::new((panel_x, panel_y).into(), (panel_w, panel_h).into());

    let cell_w = (panel_w - GAP * 3) / 2;
    let cell_h = (panel_h - GAP * 3) / 2;

    let mut cells = Vec::with_capacity(WORKSPACE_COUNT);
    for i in 0..WORKSPACE_COUNT as i32 {
        let col = i % 2;
        let row = i / 2;
        let x = panel_x + GAP + col * (cell_w + GAP);
        let y = panel_y + GAP + row * (cell_h + GAP);
        cells.push(Rectangle::new((x, y).into(), (cell_w, cell_h).into()));
    }

    (panel, cells)
}

/// How many window rows fit inside a cell of height `cell_h`.
pub fn max_rows(cell_h: i32) -> usize {
    ((cell_h - HEADER_H - PAD) / ROW_H).max(0) as usize
}

/// Shortens `title` to at most `MAX_TITLE_CHARS` characters (with a
/// trailing "..." when it had to cut), so a long window title can't
/// spill out of its cell -- text is rasterized to a fixed-size
/// texture, so there's no clipping to lean on after the fact.
fn truncate_title(title: &str) -> String {
    if title.chars().count() <= MAX_TITLE_CHARS {
        return title.to_string();
    }
    let mut s: String = title.chars().take(MAX_TITLE_CHARS - 3).collect();
    s.push_str("...");
    s
}

/// Window titles per workspace on `output_name`, indexed by workspace.
/// Windows whose recorded workspace is outside `0..WORKSPACE_COUNT`
/// (which nothing in this codebase should produce) are left out
/// rather than shown somewhere misleading.
pub fn collect_workspaces(state: &MitosGuiState, output_name: &str) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = vec![Vec::new(); WORKSPACE_COUNT];

    for window in state.space.elements() {
        let ws = crate::wm::meta(window)
            .workspace
            .get(output_name)
            .copied()
            .unwrap_or(0);

        if ws < WORKSPACE_COUNT {
            let title = crate::shell_interaction::window_title(window)
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| "Window".to_string());
            out[ws].push(truncate_title(&title));
        }
    }

    out
}

/// The header line for workspace `index` ("Workspace 2 - 3 windows").
/// Plain ASCII separator: the font `text.rs` finds varies by system,
/// and a missing glyph renders as a blank box.
pub fn header_text(index: usize, window_count: usize) -> String {
    let noun = if window_count == 1 { "window" } else { "windows" };
    format!("Workspace {} - {} {}", index + 1, window_count, noun)
}

/// Size of the named output, falling back to the first output's size
/// (`wm::output_size`) if that name isn't found -- so the overview's
/// click hit-testing matches the output it was opened on rather than
/// always assuming the primary one.
pub fn output_size_for(state: &MitosGuiState, name: &str) -> Size<i32, Logical> {
    state
        .outputs
        .iter()
        .find(|o| o.name() == name)
        .and_then(|o| o.current_mode())
        .map(|mode| Size::from((mode.size.w, mode.size.h)))
        .unwrap_or_else(|| crate::wm::output_size(state))
}
