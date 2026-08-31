# MITOS GUI 🪟

**MITOS GUI** is the modern, Wayland-based graphical desktop environment and compositor for **mitosOS**. Built from the ground up in Rust using the [Smithay](https://github.com/Smithay/smithay) framework, it provides a highly polished, secure, and performant desktop experience featuring a signature "liquid glass" visual identity, workspace management, and native hardware acceleration.

![Rust](https://img.shields.io/badge/Rust-1.70+-orange?logo=rust)
![Wayland](https://img.shields.io/badge/Wayland-Compositor-blue?logo=wayland)
![License](https://img.shields.io/badge/License-MIT-green)

---

## 🏗️ Current Architecture & Status

MITOS GUI has advanced rapidly through the industry-level roadmap. It is no longer just a foundational compositor; it is a **functional desktop environment** with workspace management, hardware-backed rendering, system services, and native input gestures.

### ✅ What is Integrated & Working

#### 🖥️ Compositor & Window Management (Stage 1 & 4)
*   **Wayland Core:** Full Smithay 0.7 integration, XDG shell lifecycle, SHM buffers, and surface damage tracking.
*   **Workspaces:** Virtual desktop support, window-to-workspace assignment, and workspace switching via keyboard shortcuts.
*   **Window Manager:** Move, resize, snap (left/right half), maximize, fullscreen, and minimize. Includes smart focus fallback and drag-to-unmaximize.
*   **Client-Driven State:** Fully handles XDG client requests for state changes (maximize, fullscreen, minimize).

#### 🎨 Visual Shell & Renderer (Stage 2 & 3)
*   **GLES2 Rendering:** GPU-accelerated damage tracking (`OutputDamageTracker`) and frame scheduling.
*   **Liquid Glass:** Advanced theme properties including specular highlights, translucent tints, and rounded corner masks.
*   **Native Rasterization:** Custom CPU glyph rasterization (`text.rs`) and procedural icon generation (`icons.rs`)—no external UI toolkit dependencies.
*   **Workspace UI:** Visual workspace dots and overview rendering.
*   **Live Configuration:** `inotify`-based watcher (`config_watcher.rs`) for instant reloading of `~/.config/mitos/home.conf` (themes, wallpapers, panel sizes).

#### ⌨️ Input & Gestures (Stage 5)
*   **Touchpad Gestures:** Native swipe gesture engine for fluid workspace switching.
*   **Keyboard Shortcuts:** Global shortcuts for window management, workspace navigation, and app launching.
*   **Pointer Routing:** Surface-local coordinate tracking, click-to-focus, and discrete scroll wheel support.

#### ⚙️ System Services & Hardware (Stage 6 & 11)
*   **DRM/KMS Backend:** Phase 2 production backend (`drm_backend.rs`) with `libseat` integration for bare-metal TTY execution.
*   **Session Management:** Native hooks for `reboot`, `poweroff`, and `halt` (`session.rs`).
*   **Authentication:** Integrated auth prompt system for polkit/sudo requests (`auth.rs`).
*   **Notifications:** Desktop notification engine (`notifications.rs`).
*   **Supervisor Integration:** `send_ready` notification for systemd/s6 service managers (`notify.rs`).

---

## 🚀 Building & Running

### Prerequisites
MITOS GUI requires a Linux environment with the following native development libraries:


🗺️ Roadmap: What’s Next?
While the core desktop experience is functional, the following areas remain to achieve full industry-ready status:


🔥 Immediate Priorities
	1.	True Frosted Glass (Stage 3): Implement multi-pass Gaussian blur shaders and render-target background capture to replace current liquid glass tints with true frosted blur.
	2.	Multi-Monitor & HiDPI (Stage 1/2): Hotplugging displays, per-monitor fractional scaling, and independent workspace mapping per output.
	3.	Layer Shell Protocol (Stage 1): Implement  wlr-layer-shell  to support standard Wayland status bars, overlays, and lock screens.


🛠️ Mid-Term Goals
	•	System Integration (Stage 7 & 8): File manager drag-and-drop, full network/audio/bluetooth UI integration, and MIME-type handling.
	•	Security & Sandboxing (Stage 9): Strict Wayland protocol security policies, clipboard privacy controls, and screencopy permissions.
	•	Accessibility (Stage 10): High-contrast themes, screen reader hooks, and keyboard navigation focus rings.


🏭 Long-Term Vision
	•	Application Ecosystem (Stage 12): Native MITOS applications (Terminal, Settings, Text Editor) and a robust compatibility layer for existing Linux apps.
	•	Reliability & CI (Stage 14 & 15): Automated QEMU graphical testing, Wayland protocol compliance tests, and reproducible release builds.


Keep rendering logic strictly separated from Wayland protocol logic.
	4.	Refer to  INTEGRATION.md  for details on how the compositor hooks into  mitos-init  and the broader mitosOS ecosystem.


**Ubuntu/Debian:**
```bash
sudo apt-get install -y pkg-config libudev-dev libinput-dev libseat-dev \
    libgbm-dev libdrm-dev libegl1-mesa-dev libgl1-mesa-dev libpixman-1-dev \
    libxkbcommon-dev libwayland-dev wayland-protocols libxcb1-dev \
    libxcb-composite0-dev libxcb-xfixes0-dev libxcb-render0-dev \
    libxcb-shm0-dev libxcb-xkb-dev
