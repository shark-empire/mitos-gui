1. Architecture map (mitos-init → services → mitos-gui)
2. Boot integration (service unit, X-Critical, READY=1 protocol)
3. Config channel (~/.config/mitos/home.conf keys, live reload)
4. Notifications (push API → future D-Bus org.freedesktop.Notifications)
5. Status providers (status.rs ← mitos-network / audio service)
6. App integration (dock IDs ↔ XDG app-ids, .desktop discovery)
7. Session control (logout/reboot/shutdown via mitos-init signals)
8. DRM/seat handoff and VT switching
9. Testing matrix (Winit dev vs QEMU production)
10. Daemon integrations (mitos-power, mitos-bluetooth, mitos-input) -- see below


🔌 System Connections & Integration
These features map directly to the 9 points in your  INTEGRATION.md  file:
	1.	Status Providers (Point 5): Instead of relying only on keyboard keys,  mitos-service  should eventually poll hardware brightness via  /sys/class/backlight/  and push updates to  state.osd .
	2.	Config Channel (Point 3): When the user toggles Night Light via a Hot Corner or  mitos-settings , the compositor should write  night_light = true  to  ~/.config/mitos/home.conf . Your  ConfigWatcher  will pick it up, trigger  reload_configuration() , and apply the tint instantly.
	3.	D-Bus Notifications (Point 4): If an external application tries to set the system volume via PulseAudio/PipeWire,  mitos-network  or your audio service can emit a signal that  mitos-gui  catches, allowing it to show the OSD pill even if the volume was changed via an external tool.


## 10. Daemon integrations (mitos-power, mitos-bluetooth, mitos-input)

mitos-gui talks to three daemons over their own Unix sockets. All three are
optional: each client connects lazily, retries with backoff (1 s doubling to
30 s), and its feature falls back or simply doesn't appear while the daemon is
missing. There are no extra threads or channels -- the sockets are
non-blocking and read once per frame from `MitosGuiState::poll_services()`
(`src/services.rs`, transport in `src/ipc_line.rs`). While mitos-power is
connected, sysfs battery polling stops.

| Daemon | Socket (env override) | Wire format | Access needed |
|---|---|---|---|
| mitos-power | `/run/mitos/power.sock` (`MITOS_POWER_SOCKET`) | JSON lines | socket is `0660 root:power`: run mitos-gui as a member of `power`; suspend/reboot/poweroff need that group (or root) too |
| mitos-bluetooth | `/run/mitos/bluetooth.sock`, then `/tmp/mitos-bluetooth.sock` (`MITOS_BLUETOOTH_SOCKET`) | JSON lines | `power_on`/`power_off`/pairing need the daemon's own uid |
| mitos-input | `/run/mitos/input.sock` (`MITOS_INPUT_SOCKET`) | text lines | socket is `0600`: same user as mitos-input |

Whatever the socket, the server must be root or the same uid as mitos-gui
(checked with `SO_PEERCRED`); anything else is refused.

### What each one drives

**mitos-power**
- Battery percentage, charging state and time left (tray icon, Quick Settings
  info line), pushed by the daemon.
- Brightness: the Quick Settings slider sends `SetBrightness`. The brightness
  keys are normally handled by mitos-power's own key watcher (the OSD follows
  its `BrightnessChanged`); if no change arrives within 400 ms, or the daemon
  is down, the shell steps the level itself (then via `brightnessctl`/`light`).
- Power profile button (Quick Settings): cycles `ListProfiles` order.
- Idle: input is forwarded as `ReportActivity` (every 5 s, immediately when
  waking). `IdleStateChanged` dim draws a 55% black layer, off a fully black
  one; the input that wakes a blanked screen is swallowed.
- A fullscreen window holds an idle inhibitor (heuristic, until
  `zwp_idle_inhibit_v1` is wired).
- `XF86Sleep` suspends; `Super+Shift+R` / `Super+Shift+P` reboot / power off
  through the daemon (honouring inhibitors), falling back to signals to PID 1.
- Toasts: low battery (20/10/5%), thermal warnings, scheduled shutdown, and
  refused actions. `ResumeFinished` forces a redraw and re-reads status.

**mitos-bluetooth**
- Tray icon (on / device connected); Quick Settings toggle with a caption
  listing connected devices and their batteries.
- Toasts for connect/disconnect and low device battery (15%).
- Pairing: just-works and numeric-comparison requests open a confirm/reject
  prompt (a context menu); a passkey to type on the other device is shown as a
  toast; PIN entry is not supported yet. Requests while locked are rejected.

**mitos-input**
- Hot-plug toasts; the on-screen keyboard is raised when a touchscreen is
  present but no keyboard is (`MITOS_GUI_AUTO_OSK=0` disables this).
- Touchscreen swipes with 3+ fingers: up/down opens/closes the overview,
  left/right switches workspace (touchpad swipes keep using libinput).
- Deliberately *not* used: the `KEY`/`POINTER` relay (libinput already
  delivers those, without an extra hop) and `HOTKEY` (keyboard.rs already
  matches the same shortcuts, so both would fire).

### Not done yet
- Touch to pointer emulation (touch events are still ignored by
  `input.rs`; mitos-input's `TOUCH` stream could drive it).
- Real DPMS / panel power-off: dim and blank are a software overlay.
- `zwp_idle_inhibit_v1` for apps that ask to keep the screen on.
- A PIN/passkey entry UI for Bluetooth pairing.
- mitos-bluetooth's `subscribe` has no event filter, so the daemon serializes
  every HID report for us only for the client to drop it. A filter parameter
  there would remove that work on both sides.
