1. Architecture map (mitos-init → services → mitos-gui)
2. Boot integration (service unit, X-Critical, READY=1 protocol)
3. Config channel (~/.config/mitos/home.conf keys, live reload)
4. Notifications (push API → future D-Bus org.freedesktop.Notifications)
5. Status providers (status.rs ← mitos-network / audio service)
6. App integration (dock IDs ↔ XDG app-ids, .desktop discovery)
7. Session control (logout/reboot/shutdown via mitos-init signals)
8. DRM/seat handoff and VT switching
9. Testing matrix (Winit dev vs QEMU production)


🔌 System Connections & Integration
These features map directly to the 9 points in your  INTEGRATION.md  file:
	1.	Status Providers (Point 5): Instead of relying only on keyboard keys,  mitos-service  should eventually poll hardware brightness via  /sys/class/backlight/  and push updates to  state.osd .
	2.	Config Channel (Point 3): When the user toggles Night Light via a Hot Corner or  mitos-settings , the compositor should write  night_light = true  to  ~/.config/mitos/home.conf . Your  ConfigWatcher  will pick it up, trigger  reload_configuration() , and apply the tint instantly.
	3.	D-Bus Notifications (Point 4): If an external application tries to set the system volume via PulseAudio/PipeWire,  mitos-network  or your audio service can emit a signal that  mitos-gui  catches, allowing it to show the OSD pill even if the volume was changed via an external tool.
