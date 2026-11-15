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
const LAYER_UNIFORM_SIZE: usize = 32;

/// One picture to draw into the frame.
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
    pipeline: RenderPipeline,
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
        let pipeline = create_pipeline(&device, &bind_group_layout);
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

        Ok(Self {
            device,
            queue,
            pipeline,
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
        })
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
    }

    /// Draws `layers` over a black frame, the first one at the bottom, and
    /// writes the result into `pixels` as opaque RGBA bytes, top row first.
    pub fn render(&mut self, layers: &[LayerDraw<'_>], pixels: &mut Vec<u8>) -> Result<(), String> {
        // Pictures of layers that are gone are released.
        self.layers
            .retain(|key, _| layers.iter().any(|layer| layer.key == *key));
        for layer in layers {
            self.upload(layer)?;
        }

        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("vizx frame"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &self.frame_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color::BLACK),
                        store: StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            for layer in layers {
                if let Some(uploaded) = self.layers.get(&layer.key) {
                    pass.set_bind_group(0, &uploaded.bind_group, &[]);
                    pass.draw(0..4, 0..1);
                }
            }
        }
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

        // The `Layer` struct of `compositor.wgsl`, in its layout.
        let (frame_width, frame_height) = (self.width as f32, self.height as f32);
        let uniforms = [
            layer.center[0] * frame_width,
            layer.center[1] * frame_height,
            layer.size[0] * frame_width / 2.0,
            layer.size[1] * frame_height / 2.0,
            frame_width,
            frame_height,
            layer.rotation_degrees.to_radians(),
            layer.opacity.clamp(0.0, 1.0),
        ];
        let bytes: Vec<u8> = uniforms
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        self.queue.write_buffer(&uploaded.uniforms, 0, &bytes);
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

fn create_pipeline(device: &Device, bind_group_layout: &BindGroupLayout) -> RenderPipeline {
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
                // "Over": the new layer plus what shows through it.
                blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
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
        }
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
