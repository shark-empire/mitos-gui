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
*   **Session Management:** Native hooks for `reboot`, `poweroff`, and `halt` (`session.rs`).
*   **Authentication:** Integrated auth prompt system for polkit/sudo requests (`auth.rs`).
*   **Supervisor Integration:** `send_ready` notification for systemd/s6 service managers (`notify.rs`).

---

## 🗺️ Roadmap: What’s Next?

While the core desktop experience is functional and multi-monitor ready, the following areas remain to achieve full industry-ready status:

### 🔥 Immediate Priorities
1.  **True Frosted Glass Integration (Stage 3):** The GLSL cross-blur shaders are written. The next step is wrapping them in a fully compliant Smithay 0.7 `RenderElement` to replace current liquid glass tints with true background-sampled frosted blur.
2.  **Layer Shell Protocol (Stage 1):** Implement `wlr-layer-shell` to support standard Wayland status bars (like `waybar`), overlays, and lock screens.
3.  **Fractional HiDPI Scaling (Stage 2):** Refine per-monitor fractional scaling (e.g., 1.5x on a 4K monitor) and ensure text rasterization remains crisp across mixed-DPI setups.

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
