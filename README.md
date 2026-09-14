# MITOS GUI 🪟

**MITOS GUI** is the modern, Wayland-based graphical desktop environment and compositor for **mitosOS**. Built from the ground up in Rust using the [Smithay](https://github.com/Smithay/smithay) framework, it provides a highly polished, secure, and performant desktop experience featuring a signature "liquid glass" visual identity, workspace management, and native hardware acceleration.

![Rust](https://img.shields.io/badge/Rust-1.70+-orange?logo=rust)
![Wayland](https://img.shields.io/badge/Wayland-Compositor-blue?logo=wayland)
![License](https://img.shields.io/badge/License-MIT-green)

---



## 🏗️ Current Architecture & Status

MITOS GUI has advanced rapidly through the industry-level roadmap. It is no longer just a foundational compositor; it is a **fully multi-monitor aware, functional desktop environment** with system-level services, hardware-backed rendering, and native input gestures.

### ✅ What is Integrated & Working

#### 🖥️ Compositor & Window Management (Stage 1, 2 & 4)
*   **Wayland Core:** Full Smithay 0.7 integration, XDG shell lifecycle, SHM buffers, and surface damage tracking.
*   **Multi-Monitor & Hotplugging:** Dynamic DRM output management. Supports plugging/unplugging HDMI/DisplayPort cables on the fly, automatically mapping new monitors into the global `Space` with correct X-axis offsets.
*   **Independent Workspaces:** Per-monitor workspace mapping (via `HashMap<String, usize>`). Workspace 1 on your laptop screen does not interfere with Workspace 3 on your external monitor.
*   **Multi-Monitor Aware WM:** Maximize, fullscreen, and window snapping (left/right half) dynamically calculate geometry based on the specific monitor the window currently resides on.
*   **Client-Driven State:** Fully handles XDG client requests for state changes.

#### 🎨 Visual Shell & Renderer (Stage 2 & 3)
*   **GLES2 Rendering:** GPU-accelerated damage tracking (`OutputDamageTracker`) and frame scheduling.
*   **Liquid Glass:** Advanced theme properties including specular highlights, translucent tints, and rounded corner masks. *(True multi-pass Gaussian frosted blur shaders are written and ready for `RenderElement` integration).*
*   **Native Rasterization:** Custom CPU glyph rasterization (`text.rs`) and procedural icon generation (`icons.rs`)—no external UI toolkit dependencies.
*   **Live Configuration:** `inotify`-based watcher (`config_watcher.rs`) for instant reloading of `~/.config/mitos/home.conf`.

#### ⌨️ Input & Gestures (Stage 5)
*   **Touchpad Gestures:** Native swipe gesture engine for fluid workspace switching.
*   **Keyboard Shortcuts:** Global shortcuts for window management, workspace navigation, and app launching.
*   **Pointer Routing:** Surface-local coordinate tracking, click-to-focus, and discrete scroll wheel support.

#### ⚙️ System Services & Hardware (Stage 6 & 11)
*   **Pure-Rust D-Bus Notifications:** Implemented `org.freedesktop.Notifications` using `zbus`. Third-party apps can push system notifications directly into the MITOS engine without relying on C-bindings (`libdbus`).
*   **Hardware-Accelerated Screenshots:** Native GPU framebuffer dumping (`Super + PrintScreen`). Renders the scene to an offscreen texture, reads pixels back via `ExportMem`, and saves timestamped PNGs to `~/Pictures`.
*   **DRM/KMS Backend:** Phase 3 production backend (`drm_backend.rs`) with `libseat` integration, `udev`-style hotplug polling, and bare-metal TTY execution.
*   **Session Management:** Native hooks for `reboot`, `poweroff`, and `halt` (`session.rs`); a persistent IPC connection to mitos-session (`session_ipc.rs`) for everything that requires a real trust decision -- login/lock state, locking, and privilege elevation.
*   **Authentication:** Compositor-drawn, unspoofable prompt (`auth.rs`) covering two real flows plus one local dev mock. **Lock screen:** driven by mitos-session's `ShowLockScreen`/`HideLockScreen`/`AuthFeedback`, password checked via real PAM on mitos-session's side. **Elevation prompts:** mitos-session, relaying mitos-service (the permission-policy daemon), asks the logged-in user to be verified before a privileged action proceeds (`ShowElevationPrompt`/`ElevationFeedback`/`HideElevationPrompt`); unlike the lock screen this can be declined (Escape), and more than one can be pending at once (queued, shown oldest-first). **Local mock** (`Super+Shift+A`): a hardcoded-password prompt for exercising the UI without a live mitos-session + mitos-service, unrelated to either real flow. See mitos-session's `docs/security.md` for the trust boundaries both real flows rely on.
*   **Supervisor Integration:** `send_ready` notification for systemd/s6 service managers (`notify.rs`).

---

## 🗺️ Roadmap: What’s Next?

While the core desktop experience is functional and multi-monitor ready, the following areas remain to achieve full industry-ready status:

### 🔥 Immediate Priorities
1.  **True Frosted Glass Integration (Stage 3):** The GLSL cross-blur shaders are written. The next step is wrapping them in a fully compliant Smithay 0.7 `RenderElement` to replace current liquid glass tints with true background-sampled frosted blur.
2.  **Layer Shell Protocol (Stage 1):** Implement `wlr-layer-shell` to support standard Wayland status bars (like `waybar`) and third-party overlay clients. *Not* what makes the lock screen/elevation prompt unspoofable -- that prompt is drawn directly in the compositor's own render list (`auth.rs` + `renderer::collect_auth_elements`), appended after every window so it's always on top, with no client-submitted surface involved at all. This item is about giving *other* processes a standard way to layer content, which mitos-gui's own built-in prompt has never needed.
3.  **Fractional HiDPI Scaling (Stage 2):** Refine per-monitor fractional scaling (e.g., 1.5x on a 4K monitor) and ensure text rasterization remains crisp across mixed-DPI setups.
4.  **`session_ipc` reconnection:** if mitos-session's connection drops (e.g. it crashes and is restarted by mitos-services), `poll_session_ipc` currently has no way to tell "nothing new happened this frame" apart from "the connection is gone" -- a lock screen or elevation prompt open at that moment would sit on screen indefinitely with no way to resolve, since nothing is listening for its answer anymore. Pre-existing (not introduced by the elevation work below), and shared by both flows equally; fixing it means detecting `mpsc::TryRecvError::Disconnected` in that loop, clearing `session_ipc`, and giving `AuthPrompt` a way to abandon whatever it's showing.

### 🔒 Elevation prompts (this pass)
`auth.rs`, `state.rs`, `keyboard.rs`, and `renderer.rs` now implement mitos-session's elevation flow end-to-end: showing what mitos-service is asking for (app, action, risk, duration), submitting or declining, queuing multiple concurrent prompts, and reacting to the outcome -- see mitos-session's `docs/security.md` (Elevation section) for the trust model this implements. Two things anyone picking this up should know:
*   **This depends on an updated `mitos-session`.** `Cargo.toml` pulls `mitos-session` from `https://github.com/shark-empire/mitos-session` -- the `elevation` module and the `RequestElevation`/`RespondElevation`/`ShowElevationPrompt`/etc. types this code imports only exist there once that remote has the corresponding mitos-session changes. Until then this won't build against a fresh `cargo fetch`; point `[patch]` at a local checkout with those changes in the meantime.
*   **mitos-service itself is out of scope here.** This is mitos-session's *relay* of a request mitos-service already decided needs a password -- mitos-gui has no code path that talks to mitos-service, classifies risk, or grants anything; it only ever draws what mitos-session tells it to and reports back what the user did.

### 🛠️ Mid-Term Goals
*   **System Integration (Stage 7 & 8):** File manager drag-and-drop, full network/audio/bluetooth UI integration, and MIME-type handling.
*   **Security & Sandboxing (Stage 9):** Strict Wayland protocol security policies, clipboard privacy controls, and screencopy permissions.
*   **Accessibility (Stage 10):** High-contrast themes, screen reader hooks (AT-SPI), and keyboard navigation focus rings.

### 🏭 Long-Term Vision
*   **Application Ecosystem (Stage 12):** Native MITOS applications (Terminal, Settings, Text Editor) and a robust compatibility layer for existing Linux apps.
*   **Reliability & CI (Stage 14 & 15):** Automated QEMU graphical testing, Wayland protocol compliance tests, and reproducible release builds.

---

## 🚀 Building & Running

### Prerequisites
MITOS GUI requires a Linux environment with the following native development libraries:

**Ubuntu/Debian:**
```bash
sudo apt-get install -y pkg-config libudev-dev libinput-dev libseat-dev \
    libgbm-dev libdrm-dev libegl1-mesa-dev libgl1-mesa-dev libpixman-1-dev \
    libxkbcommon-dev libwayland-dev wayland-protocols libxcb1-dev \
    libxcb-composite0-dev libxcb-xfixes0-dev libxcb-render0-dev \
    libxcb-shm0-dev libxcb-xkb-dev
