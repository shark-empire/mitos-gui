//! Stage 6: system status providers.
//!
//! Reads kernel sysfs interfaces directly — no daemons required.
//! Future: mitos-network / the audio service will push real values
//! over the MITOS config channel (see INTEGRATION.md).

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NetworkStatus {
    Offline,
    Ethernet,
    Wifi(u8), // signal 0-100
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BatteryStatus {
    pub capacity: u8,
    pub charging: bool,
}

/// Detect the best available network connection.
pub fn poll_network() -> NetworkStatus {
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else {
        return NetworkStatus::Offline;
    };

    let mut status = NetworkStatus::Offline;

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();

        if name == "lo" {
            continue;
        }

        let Ok(state) =
            std::fs::read_to_string(format!("/sys/class/net/{name}/operstate"))
        else {
            continue;
        };

        if state.trim() != "up" {
            continue;
        }

        if name.starts_with("wl") {
            // Signal quality from /proc/net/wireless comes later.
            status = NetworkStatus::Wifi(70);
        } else if status == NetworkStatus::Offline {
            status = NetworkStatus::Ethernet;
        }
    }

    status
}

/// Detect the first battery, if the machine has one.
pub fn poll_battery() -> Option<BatteryStatus> {
    let entries = std::fs::read_dir("/sys/class/power_supply").ok()?;

    for entry in entries.flatten() {
        let base = entry.path();

        let Ok(kind) = std::fs::read_to_string(base.join("type")) else {
            continue;
        };

        if kind.trim() != "Battery" {
            continue;
        }

        let capacity = std::fs::read_to_string(base.join("capacity"))
            .ok()
            .and_then(|s| s.trim().parse::<u8>().ok())
            .unwrap_or(100);

        let st = std::fs::read_to_string(base.join("status"))
            .unwrap_or_default();

        let charging = st.trim() == "Charging" || st.trim() == "Full";

        return Some(BatteryStatus { capacity, charging });
    }

    None
}
