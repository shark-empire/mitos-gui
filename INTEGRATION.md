1. Architecture map (mitos-init → services → mitos-gui)
2. Boot integration (service unit, X-Critical, READY=1 protocol)
3. Config channel (~/.config/mitos/home.conf keys, live reload)
4. Notifications (push API → future D-Bus org.freedesktop.Notifications)
5. Status providers (status.rs ← mitos-network / audio service)
6. App integration (dock IDs ↔ XDG app-ids, .desktop discovery)
7. Session control (logout/reboot/shutdown via mitos-init signals)
8. DRM/seat handoff and VT switching
9. Testing matrix (Winit dev vs QEMU production)
