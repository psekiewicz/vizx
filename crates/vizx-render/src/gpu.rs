use std::sync::{Arc, Mutex};

use pollster::block_on;
use wgpu::{
    Backends, Buffer, Device, DeviceDescriptor, Instance, InstanceDescriptor, Limits, MapMode,
    PollType, Queue, RequestAdapterOptions,
};

/// Connects to a graphics card without needing a window.
pub fn open_device(label: &str) -> Result<(Device, Queue), String> {
    let mut descriptor = InstanceDescriptor::new_without_display_handle();
    // `WGPU_BACKEND` overrides the default, as it does for the VizX window.
    descriptor.backends = Backends::from_env().unwrap_or_else(default_backends);
    let instance = Instance::new(descriptor);

    let adapter = block_on(instance.request_adapter(&RequestAdapterOptions::default()))
        .map_err(|error| format!("no usable graphics adapter was found: {error}"))?;
    block_on(adapter.request_device(&DeviceDescriptor {
        label: Some(label),
        // Ask only for what old and integrated graphics cards can do.
        required_limits: Limits::downlevel_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .map_err(|error| format!("the graphics adapter could not be opened: {error}"))
}

/// The same choice the VizX window makes (see `vizx-app/src/graphics.rs`):
/// no Vulkan by default on Windows, where an outdated driver can crash it.
#[cfg(target_os = "windows")]
fn default_backends() -> Backends {
    Backends::DX12 | Backends::GL
}

#[cfg(not(target_os = "windows"))]
fn default_backends() -> Backends {
    Backends::PRIMARY | Backends::GL
}

/// Waits until a picture that was copied into `buffer` has arrived, and
/// moves it into `pixels` as tightly packed RGBA rows.
///
/// The graphics card pads each row of a copied picture to `padded_row_bytes`;
/// the padding is dropped here.
pub fn read_back_rgba(
    device: &Device,
    buffer: &Buffer,
    (width, height): (u32, u32),
    padded_row_bytes: u32,
    pixels: &mut Vec<u8>,
) -> Result<(), String> {
    let slice = buffer.slice(..);
    let map_result = Arc::new(Mutex::new(None));
    {
        let map_result = Arc::clone(&map_result);
        slice.map_async(MapMode::Read, move |result| {
            if let Ok(mut slot) = map_result.lock() {
                *slot = Some(result);
            }
        });
    }
    device
        .poll(PollType::wait_indefinitely())
        .map_err(|error| format!("waiting for the graphics card failed: {error}"))?;
    match map_result.lock().ok().and_then(|mut slot| slot.take()) {
        Some(Ok(())) => {}
        Some(Err(error)) => return Err(format!("could not read the picture back: {error}")),
        None => return Err("the graphics card did not return the picture".to_owned()),
    }

    let row_bytes = (width * 4) as usize;
    pixels.clear();
    pixels.reserve(row_bytes * height as usize);
    {
        let mapped = slice
            .get_mapped_range()
            .map_err(|error| format!("could not read the picture back: {error}"))?;
        for row in mapped.chunks(padded_row_bytes as usize) {
            pixels.extend_from_slice(&row[..row_bytes]);
        }
    }
    buffer.unmap();
    Ok(())
}
