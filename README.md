# mitos-gui

Lightweight, modern Wayland compositor and desktop shell for MITOS, built
on [Smithay](https://github.com/Smithay/smithay).

The window-management and drawing policy — glass panels, blur, rounded
corners, the whole MITOS look — is deliberately *not* something Wayland
or Smithay provide for free. Smithay is a compositor framework, not a
compositor; every visual decision below Stage 3 is ours to make, in
`renderer.rs` and (later) `theme.rs`/`desktop.rs`, not something
"Wayland gives you."

## Status

**Stage 2 (renderer) complete.** `mitos-gui` is a real, runnable
compositor: it opens a window via winit, renders every mapped client
into it through a GLES2 context with damage tracking, and forwards host
keyboard/mouse input to clients (including a basic click-to-focus
policy). There's no visual identity yet — Stage 3 is what makes it look
like MITOS instead of a bare desktop — and no window management beyond
"new windows cascade, clicking raises and focuses." Every fix so far is
tracked stage-by-stage below.

## Building & running

```sh
cargo run
```

This launches `mitos-gui` as a windowed (winit) Wayland compositor —
useful for development, since it runs nested inside your existing
desktop session rather than needing a spare TTY. Point a Wayland client
at the socket it prints on startup, e.g.:

```sh
WAYLAND_DISPLAY=<socket-name-from-startup-log> weston-terminal
```

System dependencies (Debian/Ubuntu package names — see `ci.yml` for the
full list): `libwayland-dev`, `libxkbcommon-dev`, `libegl1-mesa-dev`,
`libgl1-mesa-dev`, plus the DRM/GBM/libinput/libseat dev packages Stage
5 will need.

## Roadmap

**Stage 1 — compositor foundation** ✅
Wayland socket, client connections, XDG surfaces, output, seat globals.

**Stage 2 — renderer** ✅
GLES2 GPU rendering via a winit-backed EGL context, damage tracking
(`OutputDamageTracker`), and frame scheduling (clients are told when
their last frame was presented, so well-behaved ones throttle redraws
to the output's refresh rate instead of spinning). Basic keyboard and
pointer input, including click-to-focus, came along for the ride here
too — a renderer you can't click into isn't very testable.

**Stage 3 — MITOS visual shell**
Glass panels, translucent surfaces, rounded corners, shadows, wallpaper,
top bar, launcher. All of this is renderer/shell-level compositing
policy — new render elements and shaders in `renderer.rs`, driven by
the design tokens already sitting in `theme.rs` — not a Wayland
protocol feature.

**Stage 4 — window manager**
Move, resize, maximize, minimize, workspaces, keyboard shortcuts. Builds
on `pointer.rs`'s existing click-to-focus/raise plumbing and
`keyboard.rs`'s currently-empty filter hook.

**Stage 5 — hardware**
DRM, GBM, libseat, real keyboard/mouse via libinput, real display
output, booting directly into MITOS instead of nesting inside a host
session. `input.rs`'s dispatcher and `renderer.rs`'s element collection
are both already written against the generic `InputBackend`/`GlesRenderer`
types specifically so this swap doesn't require touching either file.

## Project layout

```
src/
├── main.rs         — entry point: backend init, main loop, frame lifecycle
├── compositor.rs    — Wayland protocol handlers (XDG shell, SHM, compositor, output)
├── state.rs         — MitosGuiState: the one struct everything hangs off
├── renderer.rs       — GLES rendering, damage tracking, render-element collection
├── surface.rs        — Window <-> Space bridging helpers
├── input.rs          — raw InputEvent dispatch
├── keyboard.rs        — keyboard event -> seat forwarding
├── pointer.rs          — pointer motion/button/axis, click-to-focus
├── theme.rs            — MITOS design tokens (colors, radii, spacing)
└── animation.rs        — (Stage 3)
```
