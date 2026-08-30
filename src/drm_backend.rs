//! Stage 11 production backend — Phase 1: DRM/GBM/EGL bring-up.
//!
//! Boot path target:
//!
//!     mitos-init -> mitos-gui --drm -> /dev/dri/cardN
//!         -> GBM -> EGL -> GLES renderer
//!         -> (Phase 2) DrmCompositor vblank frame loop
//!
//! Phase 1 verifies the hardware stack with no host display server:
//! open the DRM device, enumerate connectors/modes, and construct a
//! GBM+EGL GLES renderer on top of it.

use smithay::backend::drm::{DrmDevice, DrmDeviceFd};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::gbm::GbmDevice;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::reexports::drm::control::{connector, Device as ControlDevice};

/// Result of a successful DRM bring-up.
pub struct DrmProbe {
    /// e.g. "/dev/dri/card0"
    pub node: String,

    /// Human-readable connected outputs, e.g.
    /// "HDMIA: 1920x1080@60 (preferred)"
    pub connected: Vec<String>,
}

/// Pick the first existing DRM card node.
fn pick_card() -> Option<String> {
    for i in 0..8 {
        let path = format!("/dev/dri/card{i}");

        if std::path::Path::new(&path).exists() {
            return Some(path);
        }
    }

    None
}

/// Bring up the production graphics stack and report what was found.
pub fn probe_drm() -> Result<DrmProbe, Box<dyn std::error::Error>> {
    let node = pick_card()
        .ok_or("MITOS GUI: no /dev/dri/cardN found")?;

    println!("MITOS GUI: DRM bring-up on {node}");

    // --------------------------------------------------------
    // Phase 1: direct open (MITOS runs as root, single session).
    // Phase 2 will route this through libseat for VT switching.
    // --------------------------------------------------------
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&node)?;

    // ADJUST: DrmDeviceFd::new takes an OwnedFd in smithay 0.7.
    let fd = DrmDeviceFd::new(file.into());

    let (mut drm, _event_source) = DrmDevice::new(fd.clone(), false)?;

    println!("MITOS GUI: DRM device opened");

    // --------------------------------------------------------
    // Enumerate connectors / modes
    // --------------------------------------------------------
    let res = drm.resource_handles()?;

    let mut connected = Vec::new();

    for conn in res.connectors() {
        let info = drm.connector_info(conn)?;

        if info.state() == connector::State::Connected {
            let modes = info.modes();

            let description = match modes.first() {
                Some(mode) => format!(
                    "{:?}: {}x{}@{} (preferred)",
                    info.interface(),
                    mode.size().0,
                    mode.size().1,
                    mode.vrefresh(),
                ),
                None => format!("{:?}: connected, no modes", info.interface()),
            };

            println!("MITOS GUI:   connector {description}");
            connected.push(description);
        }
    }

    if connected.is_empty() {
        return Err(
            "MITOS GUI: DRM device has no connected outputs".into()
        );
    }

    // --------------------------------------------------------
    // GBM -> EGL -> GLES
    // --------------------------------------------------------
    let gbm = GbmDevice::new(fd.clone())?;

    println!("MITOS GUI: GBM device created");

    // ADJUST: smithay 0.7 EGLDisplay::new over a GbmDevice.
    let egl_display = EGLDisplay::new(gbm.clone())?;

    println!("MITOS GUI: EGL display initialized");

    let egl_context = EGLContext::new(&egl_display, None)?;

    let _renderer = unsafe { GlesRenderer::new(egl_context)? };

    println!(
        "MITOS GUI: GLES renderer created on {node} (production path)"
    );

    Ok(DrmProbe { node, connected })
}
