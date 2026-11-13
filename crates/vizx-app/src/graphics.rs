//! Graphics backend selection. The only platform-specific code in the app.
//!
//! wgpu can talk to the GPU through several APIs ("backends"): DirectX 12,
//! Vulkan, Metal and OpenGL. VizX never names a GPU vendor; it only tells
//! wgpu which backends it may consider on each operating system, and wgpu
//! picks an adapter from those.

use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup};
use eframe::wgpu::Backends;

/// Builds the wgpu configuration used by the application window.
///
/// The `WGPU_BACKEND` environment variable (`dx12`, `vulkan`, `gl`, ...)
/// always overrides the per-platform default, which is useful for testing
/// and for working around a broken driver without rebuilding.
pub fn wgpu_configuration() -> WgpuConfiguration {
    let mut configuration = WgpuConfiguration::default();

    if let WgpuSetup::CreateNew(setup) = &mut configuration.wgpu_setup {
        setup.instance_descriptor.backends = Backends::from_env().unwrap_or_else(default_backends);
    }

    configuration
}

/// On Windows, DirectX 12 is the system's native API and OpenGL is the
/// fallback for old hardware.
///
/// Vulkan is deliberately not in the default set on Windows: Vulkan support
/// there depends entirely on the GPU driver, and outdated drivers can report
/// Vulkan as available and then crash during initialization. That happened
/// on the primary development laptop (2018 NVIDIA driver, debug build).
#[cfg(target_os = "windows")]
fn default_backends() -> Backends {
    Backends::DX12 | Backends::GL
}

/// Elsewhere (Linux: Vulkan, with OpenGL as fallback) wgpu's own default
/// set is used.
#[cfg(not(target_os = "windows"))]
fn default_backends() -> Backends {
    Backends::PRIMARY | Backends::GL
}
