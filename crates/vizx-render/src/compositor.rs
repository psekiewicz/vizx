use std::collections::HashMap;

use wgpu::{
    AddressMode, BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout,
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingResource, BindingType, BlendState,
    Buffer, BufferBindingType, BufferDescriptor, BufferUsages, Color, ColorTargetState,
    ColorWrites, CommandEncoderDescriptor, Device, Extent3d, FilterMode, FragmentState, LoadOp,
    MultisampleState, Operations, Origin3d, PipelineCompilationOptions, PipelineLayoutDescriptor,
    PrimitiveState, PrimitiveTopology, Queue, RenderPassColorAttachment, RenderPassDescriptor,
    RenderPipeline, RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, StoreOp, TexelCopyBufferInfo,
    TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureAspect, TextureDescriptor,
    TextureDimension, TextureFormat, TextureSampleType, TextureUsages, TextureView,
    TextureViewDescriptor, TextureViewDimension, VertexState,
};

use crate::gpu::{open_device, read_back_rgba};

const FRAME_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;
/// The graphics card wants rows of a copied picture to be a multiple of
/// this many bytes long.
const ROW_ALIGNMENT: u32 = 256;
/// Size in bytes of the `Layer` struct in `compositor.wgsl`.
const LAYER_UNIFORM_SIZE: usize = 80;

/// Effects drawn on a layer. Every strength is 0 (off) to 1 (strong).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LayerEffects {
    /// Soft focus.
    pub blur: f32,
    /// Red and blue shifted apart sideways, like a cheap lens.
    pub color_split: f32,
    /// Light spread around the bright parts.
    pub glow: f32,
    /// Moving film grain.
    pub grain: f32,
    /// Bands of the picture jumping sideways.
    pub glitch: f32,
    /// Darker edges.
    pub vignette: f32,
    /// Red, green and blue of a color the picture is tinted with (0 to 1).
    pub tint_color: [f32; 3],
    /// How much of the tint color: 0 none, 1 only the tint.
    pub tint: f32,
    /// The moment in seconds, which moves grain and glitches.
    pub seconds: f32,
}

impl LayerEffects {
    /// No effect at all.
    pub const NONE: Self = Self {
        blur: 0.0,
        color_split: 0.0,
        glow: 0.0,
        grain: 0.0,
        glitch: 0.0,
        vignette: 0.0,
        tint_color: [1.0, 1.0, 1.0],
        tint: 0.0,
        seconds: 0.0,
    };

    /// The values of the `Layer` struct of `compositor.wgsl` after its
    /// placement, for a frame `frame_height` pixels high.
    fn uniforms(&self, frame_height: f32) -> [f32; 12] {
        let strength = |value: f32| value.clamp(0.0, 1.0);
        [
            // Full blur is 3 % of the frame's height, full split 1 %.
            strength(self.blur) * 0.03 * frame_height,
            strength(self.color_split) * 0.01 * frame_height,
            strength(self.glow) * 1.5,
            strength(self.grain),
            strength(self.glitch),
            strength(self.vignette),
            // Kept small, so the noise in the shader stays exact.
            self.seconds.rem_euclid(3600.0),
            0.02 * frame_height,
            self.tint_color[0].clamp(0.0, 1.0),
            self.tint_color[1].clamp(0.0, 1.0),
            self.tint_color[2].clamp(0.0, 1.0),
            strength(self.tint),
        ]
    }
}

/// One picture to draw into the frame.
#[derive(Clone, Copy)]
pub struct LayerDraw<'a> {
    /// Identifies the layer between frames, so its picture stays on the
    /// graphics card instead of being sent again every frame.
    pub key: u64,
    /// Changes whenever `pixels` change. The same key with the same version
    /// is not uploaded again.
    pub version: u64,
    pub width: u32,
    pub height: u32,
    /// RGBA bytes, top row first, colors premultiplied by the opacity.
    pub pixels: &'a [u8],
    /// Middle of the layer, as fractions of the frame's width and height
    /// from its top left corner.
    pub center: [f32; 2],
    /// Width and height of the layer, as fractions of the frame's.
    pub size: [f32; 2],
    /// Turn around the middle, in degrees clockwise.
    pub rotation_degrees: f32,
    /// 0 (invisible) to 1.
    pub opacity: f32,
    /// When drawn larger than it is, show its pixels as sharp squares (for
    /// pixel art) instead of smoothing between them.
    pub sharp_pixels: bool,
    /// How the layer's colors combine with what is below it.
    pub blend: LayerBlend,
    pub effects: LayerEffects,
    /// The layer's picture is everything drawn below it so far (an effects
    /// layer: a "camera" that moves, turns and changes all the layers below
    /// at once). `pixels`, `width`, `height` and `blend` are not used. Its
    /// opacity mixes the changed picture with the unchanged one.
    pub shows_below: bool,
}

/// What an effects layer (see [`LayerDraw::shows_below`]) draws with: the
/// unchanged picture below it and the changed one, each with its placement.
struct BelowLayer {
    unchanged: (Buffer, BindGroup),
    changed: (Buffer, BindGroup),
    /// The [`Compositor::snapshot_generation`] they were made for.
    generation: u64,
}

/// How a layer's colors combine with the layers below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LayerBlend {
    /// The layer covers what is below, as far as it is opaque.
    #[default]
    Normal,
    /// Colors are added: brightens, black disappears (light, glow).
    Add,
    /// Like Add, but never brighter than white (softer light).
    Screen,
    /// Colors are multiplied: darkens, white disappears (shadows, paper).
    Multiply,
    /// The brighter of the two colors is kept.
    Lighten,
}

impl LayerBlend {
    pub const ALL: [Self; 5] = [
        Self::Normal,
        Self::Add,
        Self::Screen,
        Self::Multiply,
        Self::Lighten,
    ];

    fn state(self) -> BlendState {
        use wgpu::{BlendComponent, BlendFactor, BlendOperation};
        let component = |src_factor, dst_factor, operation| BlendComponent {
            src_factor,
            dst_factor,
            operation,
        };
        // Opacity builds up the same way in every mode.
        let alpha = component(
            BlendFactor::One,
            BlendFactor::OneMinusSrcAlpha,
            BlendOperation::Add,
        );
        let color = match self {
            Self::Normal => return BlendState::PREMULTIPLIED_ALPHA_BLENDING,
            Self::Add => component(BlendFactor::One, BlendFactor::One, BlendOperation::Add),
            // a + b - a * b
            Self::Screen => component(
                BlendFactor::OneMinusDst,
                BlendFactor::One,
                BlendOperation::Add,
            ),
            // a * b, and b where the layer is transparent
            Self::Multiply => component(
                BlendFactor::Dst,
                BlendFactor::OneMinusSrcAlpha,
                BlendOperation::Add,
            ),
            Self::Lighten => component(BlendFactor::One, BlendFactor::One, BlendOperation::Max),
        };
        BlendState { color, alpha }
    }
}

/// A layer's picture on the graphics card.
struct UploadedLayer {
    texture: Texture,
    uniforms: Buffer,
    bind_group: BindGroup,
    width: u32,
    height: u32,
    version: u64,
    sharp_pixels: bool,
}

/// Lays pictures over each other into one frame, on the graphics card.
///
/// This is the one place where a VizX picture is put together. The preview
/// uses it at the size of the preview panel and the export at the size of
/// the video, with the same layers in the same places.
pub struct Compositor {
    device: Device,
    queue: Queue,
    /// One per [LayerBlend], in the order of [LayerBlend::ALL].
    pipelines: Vec<(LayerBlend, RenderPipeline)>,
    bind_group_layout: BindGroupLayout,
    sampler: Sampler,
    /// Like sampler, but enlarged pictures keep sharp pixels.
    sharp_sampler: Sampler,
    frame_texture: Texture,
    frame_view: TextureView,
    readback_buffer: Buffer,
    width: u32,
    height: u32,
    padded_row_bytes: u32,
    layers: HashMap<u64, UploadedLayer>,
    /// Layers are drawn over nothing instead of over black.
    transparent: bool,
    /// A copy of the frame as drawn so far, which effects layers draw.
    snapshot_view: TextureView,
    snapshot: Texture,
    /// Counts up when the snapshot is made anew (on resizing), so the
    /// effects layers know to bind the new one.
    snapshot_generation: u64,
    below_layers: HashMap<u64, BelowLayer>,
}

impl Compositor {
    /// Prepares the graphics card for frames of the given size in pixels.
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("the frame size must not be zero".to_owned());
        }
        let (device, queue) = open_device("vizx compositor")?;

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("vizx layer"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX_FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipelines = LayerBlend::ALL
            .into_iter()
            .map(|blend| (blend, create_pipeline(&device, &bind_group_layout, blend)))
            .collect();
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("vizx layer sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        });
        let sharp_sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("vizx sharp layer sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Nearest,
            min_filter: FilterMode::Linear,
            ..Default::default()
        });
        let (frame_texture, frame_view, readback_buffer, padded_row_bytes) =
            create_frame(&device, width, height);
        let (snapshot, snapshot_view) = create_snapshot(&device, width, height);

        Ok(Self {
            device,
            queue,
            pipelines,
            bind_group_layout,
            sampler,
            sharp_sampler,
            frame_texture,
            frame_view,
            readback_buffer,
            width,
            height,
            padded_row_bytes,
            layers: HashMap::new(),
            transparent: false,
            snapshot,
            snapshot_view,
            snapshot_generation: 0,
            below_layers: HashMap::new(),
        })
    }

    /// Whether frames start transparent instead of black. A transparent
    /// frame comes out with colors premultiplied by the opacity, like the
    /// layers' pictures.
    pub fn set_transparent(&mut self, transparent: bool) {
        self.transparent = transparent;
    }

    /// The frame size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Changes the frame size. Layers keep their pictures.
    pub fn resize(&mut self, width: u32, height: u32) {
        if (width, height) == (self.width, self.height) || width == 0 || height == 0 {
            return;
        }
        let (frame_texture, frame_view, readback_buffer, padded_row_bytes) =
            create_frame(&self.device, width, height);
        self.frame_texture = frame_texture;
        self.frame_view = frame_view;
        self.readback_buffer = readback_buffer;
        self.padded_row_bytes = padded_row_bytes;
        self.width = width;
        self.height = height;
        let (snapshot, snapshot_view) = create_snapshot(&self.device, width, height);
        self.snapshot = snapshot;
        self.snapshot_view = snapshot_view;
        self.snapshot_generation += 1;
    }

    fn clear_color(&self) -> Color {
        if self.transparent {
            Color::TRANSPARENT
        } else {
            Color::BLACK
        }
    }

    /// Draws `layers` (none of them an effects layer) into the frame, over
    /// a cleared frame or over what it already holds.
    fn draw_layers(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        layers: &[LayerDraw<'_>],
        clear: bool,
    ) {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("vizx frame"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &self.frame_view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: if clear {
                        LoadOp::Clear(self.clear_color())
                    } else {
                        LoadOp::Load
                    },
                    store: StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        let mut current_blend = None;
        for layer in layers {
            if let Some(uploaded) = self.layers.get(&layer.key) {
                if current_blend != Some(layer.blend)
                    && let Some(pipeline) = self.pipeline(layer.blend)
                {
                    pass.set_pipeline(pipeline);
                    current_blend = Some(layer.blend);
                }
                pass.set_bind_group(0, &uploaded.bind_group, &[]);
                pass.draw(0..4, 0..1);
            }
        }
    }

    fn pipeline(&self, blend: LayerBlend) -> Option<&RenderPipeline> {
        self.pipelines
            .iter()
            .find(|(each, _)| *each == blend)
            .map(|(_, pipeline)| pipeline)
    }

    /// Replaces the frame by what the effects layer `key` makes of it.
    fn draw_effects_layer(&self, encoder: &mut wgpu::CommandEncoder, key: u64) {
        let Some(below) = self.below_layers.get(&key) else {
            return;
        };
        let size = Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        };
        encoder.copy_texture_to_texture(
            self.frame_texture.as_image_copy(),
            self.snapshot.as_image_copy(),
            size,
        );
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("vizx effects layer"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &self.frame_view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(self.clear_color()),
                    store: StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        // The unchanged picture at (1 - opacity) plus the changed one at
        // the opacity: colors are premultiplied, so adding mixes them.
        if let Some(pipeline) = self.pipeline(LayerBlend::Add) {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &below.unchanged.1, &[]);
            pass.draw(0..4, 0..1);
            pass.set_bind_group(0, &below.changed.1, &[]);
            pass.draw(0..4, 0..1);
        }
    }

    /// Makes sure the effects layer has its placements on the graphics card.
    fn prepare_effects_layer(&mut self, layer: &LayerDraw<'_>) {
        let is_current = self
            .below_layers
            .get(&layer.key)
            .is_some_and(|below| below.generation == self.snapshot_generation);
        if !is_current {
            let make = || {
                let uniforms = self.device.create_buffer(&BufferDescriptor {
                    label: Some("vizx effects layer placement"),
                    size: LAYER_UNIFORM_SIZE as u64,
                    usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let bind_group = self.bind_group(&uniforms, &self.sampler, &self.snapshot_view);
                (uniforms, bind_group)
            };
            let below = BelowLayer {
                unchanged: make(),
                changed: make(),
                generation: self.snapshot_generation,
            };
            self.below_layers.insert(layer.key, below);
        }
        let Some(below) = self.below_layers.get(&layer.key) else {
            return;
        };
        let opacity = layer.opacity.clamp(0.0, 1.0);
        let unchanged = self.uniform_bytes(&LayerDraw {
            center: [0.5, 0.5],
            size: [1.0, 1.0],
            rotation_degrees: 0.0,
            opacity: 1.0 - opacity,
            effects: LayerEffects::NONE,
            ..*layer
        });
        self.queue.write_buffer(&below.unchanged.0, 0, &unchanged);
        let changed = self.uniform_bytes(&LayerDraw { opacity, ..*layer });
        self.queue.write_buffer(&below.changed.0, 0, &changed);
    }

    fn bind_group(&self, uniforms: &Buffer, sampler: &Sampler, view: &TextureView) -> BindGroup {
        self.device.create_bind_group(&BindGroupDescriptor {
            label: Some("vizx layer"),
            layout: &self.bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(sampler),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(view),
                },
            ],
        })
    }

    /// The `Layer` struct of `compositor.wgsl` for `layer`, in its layout.
    fn uniform_bytes(&self, layer: &LayerDraw<'_>) -> Vec<u8> {
        let (frame_width, frame_height) = (self.width as f32, self.height as f32);
        let placement = [
            layer.center[0] * frame_width,
            layer.center[1] * frame_height,
            layer.size[0] * frame_width / 2.0,
            layer.size[1] * frame_height / 2.0,
            frame_width,
            frame_height,
            layer.rotation_degrees.to_radians(),
            layer.opacity.clamp(0.0, 1.0),
        ];
        let effects = layer.effects.uniforms(frame_height);
        placement
            .iter()
            .chain(&effects)
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    /// Draws `layers` over a black frame (or a transparent one, see
    /// [`Self::set_transparent`]), the first one at the bottom, and writes
    /// the result into `pixels` as RGBA bytes, top row first.
    pub fn render(&mut self, layers: &[LayerDraw<'_>], pixels: &mut Vec<u8>) -> Result<(), String> {
        // Pictures of layers that are gone are released.
        let is_wanted = |key: &u64, below: bool| {
            layers
                .iter()
                .any(|layer| layer.key == *key && layer.shows_below == below)
        };
        self.layers.retain(|key, _| is_wanted(key, false));
        self.below_layers.retain(|key, _| is_wanted(key, true));
        for layer in layers {
            if layer.shows_below {
                self.prepare_effects_layer(layer);
            } else {
                self.upload(layer)?;
            }
        }

        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor::default());
        // The layers up to each effects layer are drawn, then the effects
        // layer turns all of that into its own picture, and so on.
        let mut first_undrawn = 0;
        let mut is_cleared = false;
        for (index, layer) in layers.iter().enumerate() {
            if layer.shows_below {
                self.draw_layers(&mut encoder, &layers[first_undrawn..index], !is_cleared);
                self.draw_effects_layer(&mut encoder, layer.key);
                is_cleared = true;
                first_undrawn = index + 1;
            }
        }
        self.draw_layers(&mut encoder, &layers[first_undrawn..], !is_cleared);
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture: &self.frame_texture,
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

        read_back_rgba(
            &self.device,
            &self.readback_buffer,
            (self.width, self.height),
            self.padded_row_bytes,
            pixels,
        )
    }

    /// Makes sure the layer's picture and placement are on the graphics
    /// card.
    fn upload(&mut self, layer: &LayerDraw<'_>) -> Result<(), String> {
        let expected_bytes = layer.width as usize * layer.height as usize * 4;
        if layer.width == 0 || layer.height == 0 || layer.pixels.len() != expected_bytes {
            return Err(format!(
                "a layer of {} x {} pixels came with {} bytes instead of {expected_bytes}",
                layer.width,
                layer.height,
                layer.pixels.len()
            ));
        }

        let fits_existing = self.layers.get(&layer.key).is_some_and(|uploaded| {
            (uploaded.width, uploaded.height) == (layer.width, layer.height)
                && uploaded.sharp_pixels == layer.sharp_pixels
        });
        if !fits_existing {
            let uploaded = self.create_layer(layer.width, layer.height, layer.sharp_pixels);
            self.layers.insert(layer.key, uploaded);
        }
        let Some(uploaded) = self.layers.get_mut(&layer.key) else {
            return Ok(());
        };

        if !fits_existing || uploaded.version != layer.version {
            self.queue.write_texture(
                TexelCopyTextureInfo {
                    texture: &uploaded.texture,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                layer.pixels,
                TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(layer.width * 4),
                    rows_per_image: Some(layer.height),
                },
                Extent3d {
                    width: layer.width,
                    height: layer.height,
                    depth_or_array_layers: 1,
                },
            );
            uploaded.version = layer.version;
        }

        let bytes = self.uniform_bytes(layer);
        if let Some(uploaded) = self.layers.get(&layer.key) {
            self.queue.write_buffer(&uploaded.uniforms, 0, &bytes);
        }
        Ok(())
    }

    fn create_layer(&self, width: u32, height: u32, sharp_pixels: bool) -> UploadedLayer {
        let sampler = if sharp_pixels {
            &self.sharp_sampler
        } else {
            &self.sampler
        };
        let texture = self.device.create_texture(&TextureDescriptor {
            label: Some("vizx layer"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: FRAME_FORMAT,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        let uniforms = self.device.create_buffer(&BufferDescriptor {
            label: Some("vizx layer placement"),
            size: LAYER_UNIFORM_SIZE as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = self.device.create_bind_group(&BindGroupDescriptor {
            label: Some("vizx layer"),
            layout: &self.bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(sampler),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(&view),
                },
            ],
        });
        UploadedLayer {
            texture,
            uniforms,
            bind_group,
            width,
            height,
            sharp_pixels,
            // Anything: the picture is uploaded right after creation.
            version: 0,
        }
    }
}

/// The copy of the frame that effects layers draw from.
fn create_snapshot(device: &Device, width: u32, height: u32) -> (Texture, TextureView) {
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("vizx frame snapshot"),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: FRAME_FORMAT,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&TextureViewDescriptor::default());
    (texture, view)
}

/// The picture frames are drawn into, and the buffer they are read from.
fn create_frame(device: &Device, width: u32, height: u32) -> (Texture, TextureView, Buffer, u32) {
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("vizx frame"),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: FRAME_FORMAT,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&TextureViewDescriptor::default());
    let padded_row_bytes = (width * 4).div_ceil(ROW_ALIGNMENT) * ROW_ALIGNMENT;
    let readback_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("vizx frame readback"),
        size: u64::from(padded_row_bytes) * u64::from(height),
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    (texture, view, readback_buffer, padded_row_bytes)
}

fn create_pipeline(
    device: &Device,
    bind_group_layout: &BindGroupLayout,
    blend: LayerBlend,
) -> RenderPipeline {
    let module = device.create_shader_module(ShaderModuleDescriptor {
        label: Some("vizx compositor"),
        source: ShaderSource::Wgsl(include_str!("compositor.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some("vizx compositor"),
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some("vizx compositor"),
        layout: Some(&layout),
        vertex: VertexState {
            module: &module,
            entry_point: Some("vertex"),
            compilation_options: PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: PrimitiveState {
            // Four corners make two triangles that share an edge.
            topology: PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: MultisampleState::default(),
        fragment: Some(FragmentState {
            module: &module,
            entry_point: Some("fragment"),
            compilation_options: PipelineCompilationOptions::default(),
            targets: &[Some(ColorTargetState {
                format: FRAME_FORMAT,
                blend: Some(blend.state()),
                write_mask: ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A compositor, or `None` on a machine without any graphics adapter.
    fn compositor(width: u32, height: u32) -> Option<Compositor> {
        if open_device("test").is_err() {
            eprintln!("no graphics adapter: skipping");
            return None;
        }
        Some(Compositor::new(width, height).unwrap())
    }

    fn solid(color: [u8; 4], width: u32, height: u32) -> Vec<u8> {
        color.repeat((width * height) as usize)
    }

    fn layer(key: u64, pixels: &[u8], width: u32, height: u32) -> LayerDraw<'_> {
        LayerDraw {
            key,
            version: 0,
            width,
            height,
            pixels,
            center: [0.5, 0.5],
            size: [1.0, 1.0],
            rotation_degrees: 0.0,
            opacity: 1.0,
            sharp_pixels: false,
            blend: LayerBlend::Normal,
            effects: LayerEffects::NONE,
            shows_below: false,
        }
    }

    #[test]
    fn an_effects_layer_moves_and_changes_everything_below_it() {
        let Some(mut compositor) = compositor(64, 32) else {
            return;
        };
        let red = solid([255, 0, 0, 255], 4, 4);
        let green = solid([0, 255, 0, 255], 4, 4);
        let camera = |opacity: f32, effects: LayerEffects| LayerDraw {
            shows_below: true,
            // Moved half the frame to the right.
            center: [1.0, 0.5],
            opacity,
            effects,
            ..layer(5, &[], 0, 0)
        };
        let render = |compositor: &mut Compositor, layers: &[LayerDraw<'_>]| {
            let mut pixels = Vec::new();
            compositor.render(layers, &mut pixels).unwrap();
            pixels
        };

        // Red on the left half, moved by the camera to the right half.
        let moved = render(
            &mut compositor,
            &[left_half_red(&red), camera(1.0, LayerEffects::NONE)],
        );
        assert_near(pixel(&moved, 64, 8, 16), [0, 0, 0, 255]);
        assert_near(pixel(&moved, 64, 40, 16), [255, 0, 0, 255]);

        // At half opacity: half moved, half where it was.
        let half = render(
            &mut compositor,
            &[left_half_red(&red), camera(0.5, LayerEffects::NONE)],
        );
        assert_near(pixel(&half, 64, 8, 16), [128, 0, 0, 255]);
        assert_near(pixel(&half, 64, 40, 16), [128, 0, 0, 255]);

        // Its effects change what is below; a layer above it stays as it is.
        let tinted = render(
            &mut compositor,
            &[
                left_half_red(&red),
                camera(
                    1.0,
                    LayerEffects {
                        tint: 1.0,
                        tint_color: [0.0, 0.0, 1.0],
                        ..LayerEffects::NONE
                    },
                ),
                LayerDraw {
                    center: [0.25, 0.5],
                    size: [0.5, 1.0],
                    ..layer(2, &green, 4, 4)
                },
            ],
        );
        assert_near(pixel(&tinted, 64, 40, 16), [0, 0, 76, 255]);
        assert_near(pixel(&tinted, 64, 8, 16), [0, 255, 0, 255]);

        // Two effects layers each move what is below them.
        let twice = render(
            &mut compositor,
            &[
                left_half_red(&red),
                LayerDraw {
                    center: [0.75, 0.5],
                    ..camera(1.0, LayerEffects::NONE)
                },
                LayerDraw {
                    key: 6,
                    center: [0.75, 0.5],
                    ..camera(1.0, LayerEffects::NONE)
                },
            ],
        );
        assert_near(pixel(&twice, 64, 12, 16), [0, 0, 0, 255]);
        assert_near(pixel(&twice, 64, 40, 16), [255, 0, 0, 255]);

        // The frame size can change between frames.
        compositor.resize(32, 16);
        let small = render(
            &mut compositor,
            &[left_half_red(&red), camera(1.0, LayerEffects::NONE)],
        );
        assert_near(pixel(&small, 32, 20, 8), [255, 0, 0, 255]);
    }

    /// Red on the left half of the frame, nothing on the right.
    fn left_half_red<'a>(red: &'a [u8]) -> LayerDraw<'a> {
        LayerDraw {
            center: [0.25, 0.5],
            size: [0.5, 1.0],
            ..layer(1, red, 4, 4)
        }
    }

    #[test]
    fn effects_change_the_picture_as_named() {
        let Some(mut compositor) = compositor(64, 64) else {
            return;
        };
        let red = solid([255, 0, 0, 255], 4, 4);
        let render = |compositor: &mut Compositor, effects: LayerEffects| {
            let mut pixels = Vec::new();
            compositor
                .render(
                    &[LayerDraw {
                        effects,
                        ..left_half_red(&red)
                    }],
                    &mut pixels,
                )
                .unwrap();
            pixels
        };

        // Without effects the edge at x = 32 is sharp.
        let plain = render(&mut compositor, LayerEffects::NONE);
        assert_near(pixel(&plain, 64, 30, 32), [255, 0, 0, 255]);
        assert_near(pixel(&plain, 64, 33, 32), [0, 0, 0, 255]);

        // Blurred, the edge is a soft step.
        let soft = render(
            &mut compositor,
            LayerEffects {
                blur: 1.0,
                ..LayerEffects::NONE
            },
        );
        let edge = pixel(&soft, 64, 31, 32)[0];
        assert!((60..=230).contains(&edge), "blurred edge: {edge}");
        assert_near(pixel(&soft, 64, 8, 32), [255, 0, 0, 255]);

        // A full tint in blue turns red into a blue of red's brightness.
        let tinted = render(
            &mut compositor,
            LayerEffects {
                tint: 1.0,
                tint_color: [0.0, 0.0, 1.0],
                ..LayerEffects::NONE
            },
        );
        assert_near(pixel(&tinted, 64, 8, 32), [0, 0, 76, 255]);

        // A vignette darkens the corners, not the middle of the layer.
        let vignette = render(
            &mut compositor,
            LayerEffects {
                vignette: 1.0,
                ..LayerEffects::NONE
            },
        );
        assert!(pixel(&vignette, 64, 1, 1)[0] < 100);
        assert_near(pixel(&vignette, 64, 16, 32), [255, 0, 0, 255]);

        // Grain changes neighbouring pixels differently.
        let grainy = render(
            &mut compositor,
            LayerEffects {
                grain: 1.0,
                ..LayerEffects::NONE
            },
        );
        let reds: std::collections::HashSet<u8> =
            (0..16).map(|x| pixel(&grainy, 64, x, 20)[0]).collect();
        assert!(reds.len() > 3, "grain: {reds:?}");

        // Glitches move some bands of the picture sideways, at some moments.
        let moved = (0..24).any(|moment| {
            let glitched = render(
                &mut compositor,
                LayerEffects {
                    glitch: 1.0,
                    seconds: moment as f32 / 12.0,
                    ..LayerEffects::NONE
                },
            );
            (0..64).any(|y| pixel(&glitched, 64, 31, y)[0] < 128)
        });
        assert!(moved, "no band ever moved");
    }

    #[test]
    fn color_split_and_glow_reach_past_the_edge() {
        let Some(mut compositor) = compositor(400, 400) else {
            return;
        };
        let mut pixels = Vec::new();
        // A white square from 100 to 300 pixels, smaller than its layer, so
        // the light has room to spread inside the layer.
        let mut square = vec![0; 16 * 16 * 4];
        for y in 4..12 {
            for x in 4..12 {
                square[(y * 16 + x) * 4..][..4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        let draw = |effects| LayerDraw {
            effects,
            sharp_pixels: true,
            ..layer(1, &square, 16, 16)
        };
        compositor
            .render(
                &[draw(LayerEffects {
                    color_split: 1.0,
                    ..LayerEffects::NONE
                })],
                &mut pixels,
            )
            .unwrap();
        // Red is shifted 4 pixels to the left and blue to the right: just
        // outside the square's left edge only red is there, just outside
        // its right edge only blue.
        let left = pixel(&pixels, 400, 98, 200);
        let right = pixel(&pixels, 400, 301, 200);
        assert!(left[0] > 200 && left[2] < 50, "left: {left:?}");
        assert!(right[2] > 200 && right[0] < 50, "right: {right:?}");

        compositor
            .render(
                &[draw(LayerEffects {
                    glow: 1.0,
                    ..LayerEffects::NONE
                })],
                &mut pixels,
            )
            .unwrap();
        let near = pixel(&pixels, 400, 304, 200);
        assert!(near[0] > 20, "glow around the square: {near:?}");
    }

    #[test]
    fn blend_modes_combine_colors_as_named() {
        let Some(mut compositor) = compositor(8, 8) else {
            return;
        };
        let base = solid([100, 100, 100, 255], 2, 2);
        let top = solid([100, 200, 0, 255], 2, 2);
        let with = |compositor: &mut Compositor, blend| {
            let mut pixels = Vec::new();
            compositor
                .render(
                    &[
                        layer(1, &base, 2, 2),
                        LayerDraw {
                            blend,
                            ..layer(2, &top, 2, 2)
                        },
                    ],
                    &mut pixels,
                )
                .unwrap();
            pixel(&pixels, 8, 4, 4)
        };
        assert_near(
            with(&mut compositor, LayerBlend::Normal),
            [100, 200, 0, 255],
        );
        assert_near(with(&mut compositor, LayerBlend::Add), [200, 255, 100, 255]);
        // a + b - a*b, in 0..1: 0.39 + 0.39 - 0.15 = 0.63
        assert_near(
            with(&mut compositor, LayerBlend::Screen),
            [161, 222, 100, 255],
        );
        // a * b: 0.39 * 0.78 = 0.31
        assert_near(
            with(&mut compositor, LayerBlend::Multiply),
            [39, 78, 0, 255],
        );
        assert_near(
            with(&mut compositor, LayerBlend::Lighten),
            [100, 200, 100, 255],
        );
    }

    fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let start = ((y * width + x) * 4) as usize;
        pixels[start..start + 4].try_into().unwrap()
    }

    fn assert_near(actual: [u8; 4], expected: [u8; 4]) {
        let is_near = actual
            .iter()
            .zip(expected)
            .all(|(a, e)| (i16::from(*a) - i16::from(e)).abs() <= 3);
        assert!(is_near, "{actual:?} is not close to {expected:?}");
    }

    #[test]
    fn an_empty_frame_is_opaque_black() {
        let Some(mut compositor) = compositor(70, 40) else {
            return;
        };
        let mut pixels = Vec::new();

        compositor.render(&[], &mut pixels).unwrap();

        assert_eq!(pixels.len(), 70 * 40 * 4);
        assert!(pixels.chunks(4).all(|pixel| pixel == [0, 0, 0, 255]));
    }

    #[test]
    fn enlarged_pixel_art_keeps_sharp_pixels_when_asked() {
        let Some(mut compositor) = compositor(64, 16) else {
            return;
        };
        // Two pixels, black and white, stretched over the whole frame.
        let picture = [0, 0, 0, 255, 255, 255, 255, 255];
        let mut smooth = Vec::new();
        let mut sharp = Vec::new();

        compositor
            .render(&[layer(1, &picture, 2, 1)], &mut smooth)
            .unwrap();
        compositor
            .render(
                &[LayerDraw {
                    sharp_pixels: true,
                    ..layer(1, &picture, 2, 1)
                }],
                &mut sharp,
            )
            .unwrap();

        // Just left of the middle: grey when smoothed, still black when sharp.
        let smoothed = pixel(&smooth, 64, 28, 8);
        assert!(smoothed[0] > 40 && smoothed[0] < 215, "{smoothed:?}");
        assert_near(pixel(&sharp, 64, 28, 8), [0, 0, 0, 255]);
        assert_near(pixel(&sharp, 64, 36, 8), [255, 255, 255, 255]);
    }

    #[test]
    fn layers_are_placed_and_later_ones_cover_earlier_ones() {
        let Some(mut compositor) = compositor(64, 32) else {
            return;
        };
        let blue = solid([0, 0, 255, 255], 4, 4);
        let red = solid([255, 0, 0, 255], 4, 4);
        // Half-transparent white, premultiplied.
        let veil = solid([128, 128, 128, 128], 4, 4);
        let mut pixels = Vec::new();

        compositor
            .render(
                &[
                    layer(1, &blue, 4, 4),
                    // Red on the left half only.
                    LayerDraw {
                        center: [0.25, 0.5],
                        size: [0.5, 1.0],
                        ..layer(2, &red, 4, 4)
                    },
                    // The veil over the top half.
                    LayerDraw {
                        center: [0.5, 0.25],
                        size: [1.0, 0.5],
                        ..layer(3, &veil, 4, 4)
                    },
                ],
                &mut pixels,
            )
            .unwrap();

        assert_near(pixel(&pixels, 64, 8, 24), [255, 0, 0, 255]);
        assert_near(pixel(&pixels, 64, 56, 24), [0, 0, 255, 255]);
        // Half of the veil's white plus half of what is below it.
        assert_near(pixel(&pixels, 64, 8, 8), [255, 128, 128, 255]);
        assert_near(pixel(&pixels, 64, 56, 8), [128, 128, 255, 255]);
    }

    #[test]
    fn opacity_fades_and_rotation_turns_clockwise() {
        let Some(mut compositor) = compositor(64, 64) else {
            return;
        };
        // 2 x 1 pixels: red on the left, green on the right.
        let two_colors = [255, 0, 0, 255, 0, 255, 0, 255];
        let mut pixels = Vec::new();

        // Turned by 90 degrees clockwise, the left (red) end points up.
        compositor
            .render(
                &[LayerDraw {
                    rotation_degrees: 90.0,
                    ..layer(1, &two_colors, 2, 1)
                }],
                &mut pixels,
            )
            .unwrap();
        assert_near(pixel(&pixels, 64, 32, 4), [255, 0, 0, 255]);
        assert_near(pixel(&pixels, 64, 32, 60), [0, 255, 0, 255]);

        // At a quarter of its opacity over black, red is a quarter as bright.
        compositor
            .render(
                &[LayerDraw {
                    opacity: 0.25,
                    ..layer(1, &solid([255, 0, 0, 255], 2, 2), 2, 2)
                }],
                &mut pixels,
            )
            .unwrap();
        assert_near(pixel(&pixels, 64, 32, 32), [64, 0, 0, 255]);
    }

    #[test]
    fn a_picture_is_sent_again_only_when_its_version_changes() {
        let Some(mut compositor) = compositor(16, 16) else {
            return;
        };
        let red = solid([255, 0, 0, 255], 2, 2);
        let green = solid([0, 255, 0, 255], 2, 2);
        let mut pixels = Vec::new();

        compositor
            .render(&[layer(1, &red, 2, 2)], &mut pixels)
            .unwrap();
        assert_near(pixel(&pixels, 16, 8, 8), [255, 0, 0, 255]);

        // Same key and version: the pixels handed in are not looked at.
        compositor
            .render(&[layer(1, &green, 2, 2)], &mut pixels)
            .unwrap();
        assert_near(pixel(&pixels, 16, 8, 8), [255, 0, 0, 255]);

        compositor
            .render(
                &[LayerDraw {
                    version: 1,
                    ..layer(1, &green, 2, 2)
                }],
                &mut pixels,
            )
            .unwrap();
        assert_near(pixel(&pixels, 16, 8, 8), [0, 255, 0, 255]);
    }

    #[test]
    fn resizing_changes_the_frame_and_wrong_pixel_counts_are_refused() {
        let Some(mut compositor) = compositor(16, 16) else {
            return;
        };
        let mut pixels = Vec::new();

        compositor.resize(30, 10);
        compositor.render(&[], &mut pixels).unwrap();
        assert_eq!(compositor.size(), (30, 10));
        assert_eq!(pixels.len(), 30 * 10 * 4);

        let too_short = [0u8; 8];
        let error = compositor.render(&[layer(1, &too_short, 2, 2)], &mut pixels);
        assert!(error.unwrap_err().contains("16"));
    }
}
