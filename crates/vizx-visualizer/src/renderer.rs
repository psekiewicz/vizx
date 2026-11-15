use std::sync::{Arc, Mutex};

use pollster::block_on;
use wgpu::{
    AddressMode, Backends, BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout,
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingResource, BindingType, Buffer,
    BufferBindingType, BufferDescriptor, BufferUsages, Color, ColorTargetState, ColorWrites,
    CommandEncoderDescriptor, Device, DeviceDescriptor, ErrorFilter, Extent3d, FilterMode,
    FragmentState, Instance, InstanceDescriptor, Limits, LoadOp, MapMode, MultisampleState,
    Operations, Origin3d, PipelineCompilationOptions, PipelineLayoutDescriptor, PollType,
    PrimitiveState, Queue, RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline,
    RenderPipelineDescriptor, RequestAdapterOptions, SamplerBindingType, SamplerDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, StoreOp, TexelCopyBufferInfo,
    TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureAspect, TextureDescriptor,
    TextureDimension, TextureFormat, TextureSampleType, TextureUsages, TextureView,
    TextureViewDescriptor, TextureViewDimension, VertexState,
};

use crate::{PARAMETER_SLOTS, VisualizerPlugin};

/// How many past frames of the spectrum a shader can look back at with
/// `spectrum_history`.
pub const HISTORY_ROWS: usize = 256;
/// Width of the textures that carry the spectrum and the waveform.
const AUDIO_COLUMNS: usize = 512;
/// Size in bytes of the `Vizx` struct in `prelude.wgsl`.
const UNIFORM_SIZE: usize = 48 + PARAMETER_SLOTS * 4;
/// Pixel format of the picture: 8 bits each for red, green, blue, alpha.
const PICTURE_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;
const BYTES_PER_PIXEL: u32 = 4;
/// The graphics card wants rows of a copied picture to be a multiple of
/// this many bytes long.
const ROW_ALIGNMENT: u32 = 256;

/// Everything a shader is told about one frame.
pub struct FrameInput<'a> {
    /// Seconds since the start of the clip.
    pub time_seconds: f32,
    /// Loudness per frequency, 0 to 1, lowest frequency first.
    pub spectrum: &'a [f32],
    /// The sound wave, -1 to 1, oldest sample first.
    pub waveform: &'a [f32],
    pub volume: f32,
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    /// 1 on a beat (times its strength), fading to 0 shortly after.
    pub beat: f32,
    /// Seconds since the last beat; large before the first one.
    pub seconds_since_beat: f32,
    /// How many beats there have been so far.
    pub beat_count: f32,
    /// Beats per minute, or 0 if not known.
    pub tempo_bpm: f32,
    /// From [`VisualizerPlugin::pack_parameters`].
    pub parameters: [f32; PARAMETER_SLOTS],
}

/// Draws a plugin's shader into pictures, without a window.
///
/// Frames must be rendered in order: the shader can read its previous frame
/// and the recent history of the spectrum, and both are built up frame by
/// frame.
pub struct ShaderRenderer {
    device: Device,
    queue: Queue,
    pipeline: RenderPipeline,
    uniform_buffer: Buffer,
    audio_texture: Texture,
    history_texture: Texture,
    /// Two pictures used in turn: one is drawn into while the other, the
    /// previous frame, is read by the shader.
    target_textures: [Texture; 2],
    target_views: [TextureView; 2],
    /// `bind_groups[i]` is used when drawing into target `i`; it lets the
    /// shader read the other target.
    bind_groups: [BindGroup; 2],
    /// The finished picture is copied here to be read by the processor.
    readback_buffer: Buffer,
    width: u32,
    height: u32,
    padded_row_bytes: u32,
    frame_number: u32,
    /// Spectrum of the last [`HISTORY_ROWS`] frames, newest row first.
    history: Vec<u8>,
    /// Filled by the graphics driver if something goes wrong while drawing.
    device_error: Arc<Mutex<Option<String>>>,
}

impl ShaderRenderer {
    /// Prepares the graphics card to draw `plugin` at the given size.
    ///
    /// An error in the plugin's shader is reported here, with the line
    /// numbers of its `shader.wgsl`.
    pub fn new(plugin: &VisualizerPlugin, width: u32, height: u32) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("the picture size must not be zero".to_owned());
        }
        let (device, queue) = open_device()?;

        // Collect errors that happen later instead of letting wgpu panic.
        let device_error = Arc::new(Mutex::new(None));
        {
            let device_error = Arc::clone(&device_error);
            device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
                if let Ok(mut slot) = device_error.lock() {
                    slot.get_or_insert(error.to_string());
                }
            }));
        }

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("vizx visualizer inputs"),
            entries: &[
                layout_entry(
                    0,
                    BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                layout_entry(1, BindingType::Sampler(SamplerBindingType::Filtering)),
                layout_entry(2, FILTERABLE_TEXTURE),
                layout_entry(3, FILTERABLE_TEXTURE),
                layout_entry(4, FILTERABLE_TEXTURE),
            ],
        });

        // The plugin's code is compiled inside an "error scope": a mistake
        // in it comes back as an error value we can show to its author.
        let error_scope = device.push_error_scope(ErrorFilter::Validation);
        let pipeline = create_pipeline(&device, &bind_group_layout, plugin);
        if let Some(error) = block_on(error_scope.pop()) {
            return Err(format!(
                "{} has an error:\n{}",
                plugin.folder.join(crate::SHADER_FILE_NAME).display(),
                plain_text(&error.to_string())
            ));
        }

        let uniform_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("vizx visualizer uniforms"),
            size: UNIFORM_SIZE as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("vizx visualizer sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        });

        let data_usage = TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST;
        let audio_texture = create_texture(
            &device,
            "vizx audio",
            (AUDIO_COLUMNS as u32, 2),
            TextureFormat::R8Unorm,
            data_usage,
        );
        let history_texture = create_texture(
            &device,
            "vizx spectrum history",
            (AUDIO_COLUMNS as u32, HISTORY_ROWS as u32),
            TextureFormat::R8Unorm,
            data_usage,
        );
        let target_usage = TextureUsages::RENDER_ATTACHMENT
            | TextureUsages::TEXTURE_BINDING
            | TextureUsages::COPY_SRC;
        let target_textures = [0, 1].map(|_| {
            create_texture(
                &device,
                "vizx picture",
                (width, height),
                PICTURE_FORMAT,
                target_usage,
            )
        });
        let view = |texture: &Texture| texture.create_view(&TextureViewDescriptor::default());
        let target_views = [view(&target_textures[0]), view(&target_textures[1])];
        let audio_view = view(&audio_texture);
        let history_view = view(&history_texture);

        let bind_group = |previous_frame: &TextureView| {
            device.create_bind_group(&BindGroupDescriptor {
                label: Some("vizx visualizer inputs"),
                layout: &bind_group_layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: uniform_buffer.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::Sampler(&sampler),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::TextureView(&audio_view),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::TextureView(&history_view),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: BindingResource::TextureView(previous_frame),
                    },
                ],
            })
        };
        // Drawing into target 0 reads target 1 as the previous frame, and
        // the other way round.
        let bind_groups = [bind_group(&target_views[1]), bind_group(&target_views[0])];

        let padded_row_bytes = (width * BYTES_PER_PIXEL).div_ceil(ROW_ALIGNMENT) * ROW_ALIGNMENT;
        let readback_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("vizx picture readback"),
            size: u64::from(padded_row_bytes) * u64::from(height),
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let renderer = Self {
            device,
            queue,
            pipeline,
            uniform_buffer,
            audio_texture,
            history_texture,
            target_textures,
            target_views,
            bind_groups,
            readback_buffer,
            width,
            height,
            padded_row_bytes,
            frame_number: 0,
            history: vec![0; AUDIO_COLUMNS * HISTORY_ROWS],
            device_error,
        };
        renderer.clear_targets();
        renderer.take_device_error()?;
        Ok(renderer)
    }

    /// Draws the next frame and writes it into `pixels` as RGBA bytes, top
    /// row first. The fourth byte is the opacity, and the color bytes are
    /// already multiplied by it ("premultiplied alpha").
    pub fn render(&mut self, input: &FrameInput<'_>, pixels: &mut Vec<u8>) -> Result<(), String> {
        let target = (self.frame_number % 2) as usize;

        self.queue
            .write_buffer(&self.uniform_buffer, 0, &self.uniform_bytes(input));
        self.upload_audio(input);

        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("vizx visualizer"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &self.target_views[target],
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color::TRANSPARENT),
                        store: StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_groups[target], &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture: &self.target_textures[target],
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            TexelCopyBufferInfo {
                buffer: &self.readback_buffer,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_row_bytes),
                    rows_per_image: Some(self.height),
                },
            },
            Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        self.read_back(pixels)?;
        self.take_device_error()?;
        self.frame_number += 1;
        Ok(())
    }

    /// The bytes of the `Vizx` struct in `prelude.wgsl`, in its layout.
    fn uniform_bytes(&self, input: &FrameInput<'_>) -> Vec<u8> {
        let header = [
            self.width as f32,
            self.height as f32,
            input.time_seconds,
            self.frame_number as f32,
            input.volume,
            input.bass,
            input.mid,
            input.treble,
            input.beat,
            input.seconds_since_beat,
            input.beat_count,
            input.tempo_bpm,
        ];
        header
            .iter()
            .chain(&input.parameters)
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    /// Sends this frame's spectrum and waveform to the graphics card and
    /// adds the spectrum to the history.
    fn upload_audio(&mut self, input: &FrameInput<'_>) {
        let spectrum_row = to_bytes(input.spectrum, |level| level);
        // The waveform ranges from -1 to 1; bytes can only hold 0 to 1.
        let waveform_row = to_bytes(input.waveform, |sample| sample * 0.5 + 0.5);

        // Newest row first: move everything one row down, then put the new
        // spectrum on top.
        self.history
            .copy_within(..AUDIO_COLUMNS * (HISTORY_ROWS - 1), AUDIO_COLUMNS);
        self.history[..AUDIO_COLUMNS].copy_from_slice(&spectrum_row);

        let audio = [spectrum_row, waveform_row].concat();
        write_rows(&self.queue, &self.audio_texture, &audio, 2);
        write_rows(
            &self.queue,
            &self.history_texture,
            &self.history,
            HISTORY_ROWS as u32,
        );
    }

    /// Waits for the picture copy to finish and moves it into `pixels`.
    fn read_back(&self, pixels: &mut Vec<u8>) -> Result<(), String> {
        let slice = self.readback_buffer.slice(..);
        let map_result = Arc::new(Mutex::new(None));
        {
            let map_result = Arc::clone(&map_result);
            slice.map_async(MapMode::Read, move |result| {
                if let Ok(mut slot) = map_result.lock() {
                    *slot = Some(result);
                }
            });
        }
        self.device
            .poll(PollType::wait_indefinitely())
            .map_err(|error| format!("waiting for the graphics card failed: {error}"))?;
        match map_result.lock().ok().and_then(|mut slot| slot.take()) {
            Some(Ok(())) => {}
            Some(Err(error)) => return Err(format!("could not read the picture back: {error}")),
            None => return Err("the graphics card did not return the picture".to_owned()),
        }

        let row_bytes = (self.width * BYTES_PER_PIXEL) as usize;
        pixels.clear();
        pixels.reserve(row_bytes * self.height as usize);
        {
            let mapped = slice
                .get_mapped_range()
                .map_err(|error| format!("could not read the picture back: {error}"))?;
            // Drop the padding at the end of each row.
            for row in mapped.chunks(self.padded_row_bytes as usize) {
                pixels.extend_from_slice(&row[..row_bytes]);
            }
        }
        self.readback_buffer.unmap();
        Ok(())
    }

    /// Makes both pictures fully transparent, so the first frame's
    /// "previous frame" is defined.
    fn clear_targets(&self) {
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor::default());
        for view in &self.target_views {
            encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("vizx clear"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color::TRANSPARENT),
                        store: StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        self.queue.submit([encoder.finish()]);
    }

    fn take_device_error(&self) -> Result<(), String> {
        match self
            .device_error
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
        {
            Some(error) => Err(format!("the graphics card reported an error: {error}")),
            None => Ok(()),
        }
    }
}

/// Replaces the line-drawing characters wgpu decorates its error messages
/// with. Most user interface fonts cannot show them.
fn plain_text(message: &str) -> String {
    message
        .chars()
        .map(|character| match character {
            '\u{2500}'..='\u{257F}' => '|',
            other => other,
        })
        .collect()
}

const FILTERABLE_TEXTURE: BindingType = BindingType::Texture {
    sample_type: TextureSampleType::Float { filterable: true },
    view_dimension: TextureViewDimension::D2,
    multisampled: false,
};

fn layout_entry(binding: u32, ty: BindingType) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::FRAGMENT,
        ty,
        count: None,
    }
}

/// Connects to a graphics card without needing a window.
fn open_device() -> Result<(Device, Queue), String> {
    let mut descriptor = InstanceDescriptor::new_without_display_handle();
    // `WGPU_BACKEND` overrides the default, as it does for the VizX window.
    descriptor.backends = Backends::from_env().unwrap_or_else(default_backends);
    let instance = Instance::new(descriptor);

    let adapter = block_on(instance.request_adapter(&RequestAdapterOptions::default()))
        .map_err(|error| format!("no usable graphics adapter was found: {error}"))?;
    block_on(adapter.request_device(&DeviceDescriptor {
        label: Some("vizx visualizer"),
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

fn create_pipeline(
    device: &Device,
    bind_group_layout: &BindGroupLayout,
    plugin: &VisualizerPlugin,
) -> RenderPipeline {
    let module = device.create_shader_module(ShaderModuleDescriptor {
        label: Some("vizx visualizer shader"),
        source: ShaderSource::Wgsl(plugin.full_shader_source().into()),
    });
    let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some("vizx visualizer"),
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some("vizx visualizer"),
        layout: Some(&layout),
        vertex: VertexState {
            module: &module,
            entry_point: Some("vizx_vertex"),
            compilation_options: PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: MultisampleState::default(),
        fragment: Some(FragmentState {
            module: &module,
            entry_point: Some("vizx_fragment"),
            compilation_options: PipelineCompilationOptions::default(),
            targets: &[Some(ColorTargetState {
                format: PICTURE_FORMAT,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_texture(
    device: &Device,
    label: &str,
    (width, height): (u32, u32),
    format: TextureFormat,
    usage: TextureUsages,
) -> Texture {
    device.create_texture(&TextureDescriptor {
        label: Some(label),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

/// Replaces the content of a one-byte-per-pixel texture that is
/// [`AUDIO_COLUMNS`] wide.
fn write_rows(queue: &Queue, texture: &Texture, bytes: &[u8], rows: u32) {
    queue.write_texture(
        TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        bytes,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(AUDIO_COLUMNS as u32),
            rows_per_image: Some(rows),
        },
        Extent3d {
            width: AUDIO_COLUMNS as u32,
            height: rows,
            depth_or_array_layers: 1,
        },
    );
}

/// Stretches `values` to [`AUDIO_COLUMNS`] entries and stores each as a
/// byte, after `to_unit` has brought it into the range 0 to 1.
fn to_bytes(values: &[f32], to_unit: impl Fn(f32) -> f32) -> Vec<u8> {
    (0..AUDIO_COLUMNS)
        .map(|column| {
            let value = if values.is_empty() {
                0.0
            } else {
                values[column * values.len() / AUDIO_COLUMNS]
            };
            (to_unit(value).clamp(0.0, 1.0) * 255.0).round() as u8
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::Manifest;

    fn plugin(shader_source: &str) -> VisualizerPlugin {
        VisualizerPlugin {
            manifest: Manifest {
                name: "Test".to_owned(),
                author: String::new(),
                description: String::new(),
                parameters: Vec::new(),
            },
            shader_source: shader_source.to_owned(),
            folder: PathBuf::from("test-plugin"),
        }
    }

    fn input<'a>(spectrum: &'a [f32], waveform: &'a [f32]) -> FrameInput<'a> {
        FrameInput {
            time_seconds: 0.0,
            spectrum,
            waveform,
            volume: 0.0,
            bass: 0.0,
            mid: 0.0,
            treble: 0.0,
            beat: 0.0,
            seconds_since_beat: 0.0,
            beat_count: 0.0,
            tempo_bpm: 0.0,
            parameters: [0.0; PARAMETER_SLOTS],
        }
    }

    /// A renderer, or `None` on a machine without any graphics adapter
    /// (some build servers), where these tests have nothing to check.
    fn renderer(shader_source: &str, size: (u32, u32)) -> Option<Result<ShaderRenderer, String>> {
        if open_device().is_err() {
            eprintln!("no graphics adapter: skipping");
            return None;
        }
        Some(ShaderRenderer::new(&plugin(shader_source), size.0, size.1))
    }

    fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let start = ((y * width + x) * 4) as usize;
        pixels[start..start + 4].try_into().unwrap()
    }

    #[test]
    fn values_become_bytes_across_the_full_width() {
        let bytes = to_bytes(&[0.0, 1.0], |value| value);
        assert_eq!(bytes.len(), AUDIO_COLUMNS);
        assert_eq!((bytes[0], bytes[AUDIO_COLUMNS - 1]), (0, 255));

        // The waveform's -1..1 is stored as 0..1; out-of-range is clamped.
        let waveform = to_bytes(&[-1.0, 0.0, 5.0], |sample| sample * 0.5 + 0.5);
        assert_eq!(
            (
                waveform[0],
                waveform[AUDIO_COLUMNS / 2],
                waveform[AUDIO_COLUMNS - 1]
            ),
            (0, 128, 255)
        );
        assert!(to_bytes(&[], |value| value).iter().all(|byte| *byte == 0));
    }

    #[test]
    fn uv_starts_at_the_top_left() {
        // Red grows to the right, green grows downwards.
        let Some(renderer) = renderer(
            "fn visualize(uv: vec2f) -> vec4f { return vec4f(uv.x, uv.y, 0.0, 1.0); }",
            // An odd width, so rows need padding when read back.
            (70, 40),
        ) else {
            return;
        };
        let mut renderer = renderer.unwrap();
        let mut pixels = Vec::new();

        renderer.render(&input(&[], &[]), &mut pixels).unwrap();

        assert_eq!(pixels.len(), 70 * 40 * 4);
        let top_left = pixel(&pixels, 70, 0, 0);
        let top_right = pixel(&pixels, 70, 69, 0);
        let bottom_left = pixel(&pixels, 70, 0, 39);
        assert!(top_left[0] < 10 && top_left[1] < 10, "{top_left:?}");
        assert!(top_right[0] > 245 && top_right[1] < 10, "{top_right:?}");
        assert!(
            bottom_left[0] < 10 && bottom_left[1] > 245,
            "{bottom_left:?}"
        );
        assert_eq!(top_left[3], 255);
    }

    #[test]
    fn shader_sees_spectrum_waveform_history_and_its_previous_frame() {
        // Red: spectrum at the pixel's x. Green: the waveform's rightmost
        // value. Blue: the spectrum one step back in history at x = 0.
        // Alpha is unused, so the previous frame is checked through red:
        // the right half shows what the left half showed a frame ago.
        let Some(renderer) = renderer(
            "fn visualize(uv: vec2f) -> vec4f {
                var red = spectrum(uv.x);
                if uv.x > 0.5 { red = previous_frame(vec2f(uv.x - 0.5, uv.y)).r; }
                return vec4f(red, waveform(1.0) * 0.5 + 0.5, spectrum_history(0.0, 1.5 / 256.0), 1.0);
            }",
            (64, 8),
        ) else {
            return;
        };
        let mut renderer = renderer.unwrap();
        let mut pixels = Vec::new();

        // Frame 0: loud on the left of the spectrum, wave at its top.
        let loud_left: Vec<f32> = (0..512)
            .map(|bin| if bin < 128 { 1.0 } else { 0.0 })
            .collect();
        renderer
            .render(&input(&loud_left, &[1.0; 512]), &mut pixels)
            .unwrap();
        let left = pixel(&pixels, 64, 4, 4);
        let quiet = pixel(&pixels, 64, 28, 4);
        assert!(left[0] > 245 && left[1] > 245, "{left:?}");
        assert!(quiet[0] < 10, "{quiet:?}");
        // No history and no previous frame yet.
        assert!(left[2] < 10 && pixel(&pixels, 64, 36, 4)[0] < 10);

        // Frame 1: silence. The spectrum is gone, but the history and the
        // previous frame remember it.
        renderer
            .render(&input(&[0.0; 512], &[-1.0; 512]), &mut pixels)
            .unwrap();
        let left = pixel(&pixels, 64, 4, 4);
        assert!(left[0] < 10 && left[1] < 10, "{left:?}");
        assert!(left[2] > 245, "history: {left:?}");
        let echoed = pixel(&pixels, 64, 36, 4);
        assert!(echoed[0] > 245, "previous frame: {echoed:?}");
    }

    #[test]
    fn opacity_is_kept_and_the_color_is_multiplied_by_it() {
        // Left half: transparent. Right half: white at half opacity. The
        // previous frame is read back unchanged, so a second frame that
        // only repeats it gives the same picture.
        let Some(renderer) = renderer(
            "fn visualize(uv: vec2f) -> vec4f {
                if vizx.frame > 0.5 { return previous_frame(uv); }
                if uv.x < 0.5 { return vec4f(1.0, 1.0, 1.0, 0.0); }
                return vec4f(1.0, 1.0, 1.0, 0.5);
            }",
            (32, 8),
        ) else {
            return;
        };
        let mut renderer = renderer.unwrap();
        let mut pixels = Vec::new();

        for _frame in 0..2 {
            renderer.render(&input(&[], &[]), &mut pixels).unwrap();

            assert_eq!(pixel(&pixels, 32, 4, 4), [0, 0, 0, 0]);
            let half = pixel(&pixels, 32, 28, 4);
            assert!(
                half.iter().all(|part| (126..=130).contains(part)),
                "{half:?}"
            );
        }
    }

    #[test]
    fn light_on_black_becomes_transparent_black() {
        let Some(renderer) = renderer(
            "fn visualize(uv: vec2f) -> vec4f {
                if uv.x < 0.5 { return from_light(vec3f(0.0)); }
                return from_light(vec3f(0.5, 0.25, 0.0));
            }",
            (32, 8),
        ) else {
            return;
        };
        let mut pixels = Vec::new();
        renderer
            .unwrap()
            .render(&input(&[], &[]), &mut pixels)
            .unwrap();

        assert_eq!(pixel(&pixels, 32, 4, 4), [0, 0, 0, 0]);
        // The light itself is unchanged; its brightest part is the opacity.
        let lit = pixel(&pixels, 32, 28, 4);
        assert!(
            (126..=130).contains(&lit[0]) && (62..=66).contains(&lit[1]),
            "{lit:?}"
        );
        assert!(lit[2] == 0 && (126..=130).contains(&lit[3]), "{lit:?}");
    }

    /// Every plugin that ships with VizX must load, compile and draw.
    #[test]
    fn bundled_plugins_compile_and_draw_something() {
        if open_device().is_err() {
            eprintln!("no graphics adapter: skipping");
            return;
        }
        let bundled =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/visualizers");
        let mut checked = 0;
        for folder in std::fs::read_dir(&bundled).unwrap().flatten() {
            let plugin = VisualizerPlugin::load(&folder.path()).unwrap();
            let name = plugin.manifest.name.clone();
            let mut renderer = ShaderRenderer::new(&plugin, 96, 54)
                .unwrap_or_else(|error| panic!("{name}: {error}"));

            // A loud, full spectrum and a wave: every plugin shows something.
            let wave: Vec<f32> = (0..512).map(|point| (point as f32 * 0.1).sin()).collect();
            let mut frame = input(&[0.9; 512], &wave);
            (frame.volume, frame.bass, frame.mid, frame.treble) = (0.9, 0.9, 0.9, 0.9);
            frame.parameters = plugin.pack_parameters(&crate::ParameterValues::new());
            let mut pixels = Vec::new();
            for _ in 0..3 {
                renderer.render(&frame, &mut pixels).unwrap();
            }

            let is_lit = pixels
                .chunks(4)
                .any(|pixel| pixel[..3].iter().any(|part| *part > 40));
            assert!(is_lit, "{name} drew only black");
            // No bundled plugin paints a solid background.
            let has_transparency = pixels.chunks(4).any(|pixel| pixel[3] < 40);
            assert!(has_transparency, "{name} has no transparent pixels");
            checked += 1;
        }
        assert!(
            checked >= 4,
            "expected the bundled plugins in {}",
            bundled.display()
        );
    }

    /// The middle of Core stays empty, so a logo below it shows through.
    #[test]
    fn the_core_visualizer_has_a_transparent_middle() {
        if open_device().is_err() {
            return;
        }
        let folder =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/visualizers/core");
        let plugin = VisualizerPlugin::load(&folder).unwrap();
        let mut renderer = ShaderRenderer::new(&plugin, 160, 90).unwrap();
        let mut frame = input(&[0.8; 512], &[0.0; 512]);
        (frame.volume, frame.bass, frame.beat) = (0.8, 0.8, 1.0);
        frame.parameters = plugin.pack_parameters(&crate::ParameterValues::new());
        let mut pixels = Vec::new();
        for _ in 0..10 {
            renderer.render(&frame, &mut pixels).unwrap();
        }

        assert_eq!(pixel(&pixels, 160, 80, 45), [0, 0, 0, 0], "the middle");
        assert!(
            pixels.chunks(4).any(|pixel| pixel[3] > 200),
            "the ring and rays are drawn"
        );
    }

    #[test]
    fn a_mistake_in_the_shader_is_reported_not_fatal() {
        let Some(renderer) = renderer(
            "fn visualize(uv: vec2f) -> vec4f { return undefined_name; }",
            (16, 16),
        ) else {
            return;
        };

        let error = renderer.err().expect("the shader must be refused");
        assert!(error.contains("shader.wgsl"), "{error}");
        assert!(error.contains("undefined_name"), "{error}");
        assert!(error.is_ascii(), "{error}");
    }
}
