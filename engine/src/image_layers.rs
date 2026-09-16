use std::num::NonZeroU64;

#[derive(Debug, Clone, Copy)]
pub struct LayerTransform {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub rotation: f32,
}

pub fn layer_quad_ndc(
    canvas_size: (f32, f32),
    surface_size: (f32, f32),
    transform: &LayerTransform,
    asset_size: (f32, f32),
    parallax_offset: (f32, f32),
) -> [[f32; 4]; 6] {
    let (canvas_w, canvas_h) = canvas_size;
    let (surface_w, surface_h) = surface_size;
    let width = if transform.width > 0.0 {
        transform.width
    } else {
        asset_size.0
    };
    let height = if transform.height > 0.0 {
        transform.height
    } else {
        asset_size.1
    };
    let center_x = transform.x + parallax_offset.0;
    let center_y = transform.y + parallax_offset.1;
    let (sin_r, cos_r) = transform.rotation.sin_cos();
    let scale = (surface_w / canvas_w).max(surface_h / canvas_h);

    let to_ndc = |local_x: f32, local_y: f32| -> (f32, f32) {
        let rotated_x = local_x * cos_r - local_y * sin_r;
        let rotated_y = local_x * sin_r + local_y * cos_r;
        let canvas_x = center_x + rotated_x;
        let canvas_y = center_y + rotated_y;
        let surface_x = (canvas_x - canvas_w / 2.0) * scale + surface_w / 2.0;
        let surface_y = (canvas_y - canvas_h / 2.0) * scale + surface_h / 2.0;
        let ndc_x = surface_x / surface_w * 2.0 - 1.0;
        let ndc_y = 1.0 - surface_y / surface_h * 2.0;
        (ndc_x, ndc_y)
    };

    let half_w = width / 2.0;
    let half_h = height / 2.0;
    let (tl_x, tl_y) = to_ndc(-half_w, -half_h);
    let (tr_x, tr_y) = to_ndc(half_w, -half_h);
    let (br_x, br_y) = to_ndc(half_w, half_h);
    let (bl_x, bl_y) = to_ndc(-half_w, half_h);

    [
        [tl_x, tl_y, 0.0, 0.0],
        [tr_x, tr_y, 1.0, 0.0],
        [br_x, br_y, 1.0, 1.0],
        [tl_x, tl_y, 0.0, 0.0],
        [br_x, br_y, 1.0, 1.0],
        [bl_x, bl_y, 0.0, 1.0],
    ]
}

pub const LAYER_SHADER_WGSL: &str = r#"
struct Uniforms { alpha: f32, _pad0: f32, _pad1: f32, _pad2: f32 };
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var t: texture_2d<f32>;
@group(0) @binding(2) var s: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>) -> VOut {
    var out: VOut;
    out.pos = vec4<f32>(position, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let c = textureSample(t, s, in.uv);
    return vec4<f32>(c.rgb, c.a * u.alpha);
}
"#;

pub struct LayerResources {
    pub bind_group: wgpu::BindGroup,
    pub vertex_buffer: wgpu::Buffer,
    pub asset_size: (f32, f32),
}

pub struct ImageLayerRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl ImageLayerRenderer {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("image-layer-shader"),
            source: wgpu::ShaderSource::Wgsl(LAYER_SHADER_WGSL.into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("image-layer-bind-group-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(16),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("image-layer-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("image-layer-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 16,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
                    ],
                }],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,

                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        Self {
            pipeline,
            bind_group_layout,
            sampler,
        }
    }

    pub fn upload_layer(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rgba8: &[u8],
    ) -> LayerResources {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("image-layer-texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba8,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.rebind_texture(device, &view, 1.0);

        let _ = texture;

        self.finish_layer_resources(device, bind_group, (width as f32, height as f32))
    }

    pub fn rebind_texture(
        &self,
        device: &wgpu::Device,
        view: &wgpu::TextureView,
        alpha: f32,
    ) -> wgpu::BindGroup {
        let uniform = [alpha, 0.0, 0.0, 0.0];
        let uniform_buffer = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("image-layer-alpha-uniform"),
                contents: bytemuck::cast_slice(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("image-layer-bind-group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    pub fn finish_layer_resources(
        &self,
        device: &wgpu::Device,
        bind_group: wgpu::BindGroup,
        asset_size: (f32, f32),
    ) -> LayerResources {
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("image-layer-vertices"),
            size: (6 * 4 * 4) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        LayerResources {
            bind_group,
            vertex_buffer,
            asset_size,
        }
    }

    pub fn update_layer_vertices(
        &self,
        queue: &wgpu::Queue,
        layer: &LayerResources,
        vertices: &[[f32; 4]; 6],
    ) {
        let flat: Vec<f32> = vertices.iter().flatten().copied().collect();
        queue.write_buffer(&layer.vertex_buffer, 0, bytemuck::cast_slice(&flat));
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, layers: &'a [LayerResources]) {
        self.draw_refs(pass, &layers.iter().collect::<Vec<_>>());
    }

    pub fn draw_refs<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, layers: &[&'a LayerResources]) {
        pass.set_pipeline(&self.pipeline);
        for layer in layers {
            pass.set_bind_group(0, &layer.bind_group, &[]);
            pass.set_vertex_buffer(0, layer.vertex_buffer.slice(..));
            pass.draw(0..6, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_canvas_centered_layer_with_matching_aspect_fills_exact_ndc_corners() {
        let canvas = (100.0, 100.0);
        let surface = (100.0, 100.0);
        let transform = LayerTransform {
            x: 50.0,
            y: 50.0,
            width: 100.0,
            height: 100.0,
            rotation: 0.0,
        };
        let quad = layer_quad_ndc(canvas, surface, &transform, (100.0, 100.0), (0.0, 0.0));

        assert!(
            (quad[0][0] - (-1.0)).abs() < 1e-5,
            "top-left ndc_x: {}",
            quad[0][0]
        );
        assert!(
            (quad[0][1] - 1.0).abs() < 1e-5,
            "top-left ndc_y: {}",
            quad[0][1]
        );
        assert!(
            (quad[2][0] - 1.0).abs() < 1e-5,
            "bottom-right ndc_x: {}",
            quad[2][0]
        );
        assert!(
            (quad[2][1] - (-1.0)).abs() < 1e-5,
            "bottom-right ndc_y: {}",
            quad[2][1]
        );
    }

    #[test]
    fn zero_size_transform_falls_back_to_asset_pixel_size() {
        let canvas = (200.0, 100.0);
        let surface = (200.0, 100.0);
        let transform = LayerTransform {
            x: 100.0,
            y: 50.0,
            width: 0.0,
            height: 0.0,
            rotation: 0.0,
        };
        let quad = layer_quad_ndc(canvas, surface, &transform, (40.0, 20.0), (0.0, 0.0));

        assert!(
            (quad[0][0] - (-0.2)).abs() < 1e-5,
            "left edge ndc_x: {}",
            quad[0][0]
        );
        assert!(
            (quad[1][0] - 0.2).abs() < 1e-5,
            "right edge ndc_x: {}",
            quad[1][0]
        );
    }

    #[test]
    fn parallax_offset_shifts_the_quad_in_canvas_space() {
        let canvas = (100.0, 100.0);
        let surface = (100.0, 100.0);
        let transform = LayerTransform {
            x: 50.0,
            y: 50.0,
            width: 20.0,
            height: 20.0,
            rotation: 0.0,
        };
        let base = layer_quad_ndc(canvas, surface, &transform, (20.0, 20.0), (0.0, 0.0));
        let shifted = layer_quad_ndc(canvas, surface, &transform, (20.0, 20.0), (10.0, 0.0));

        assert!(
            (shifted[0][0] - base[0][0] - 0.2).abs() < 1e-5,
            "base={}, shifted={}",
            base[0][0],
            shifted[0][0]
        );
    }

    #[test]
    fn aspect_fill_crops_the_taller_axis_when_canvas_and_surface_aspect_differ() {
        let canvas = (100.0, 100.0);
        let surface = (200.0, 100.0);
        let transform = LayerTransform {
            x: 50.0,
            y: 50.0,
            width: 100.0,
            height: 100.0,
            rotation: 0.0,
        };
        let quad = layer_quad_ndc(canvas, surface, &transform, (100.0, 100.0), (0.0, 0.0));
        assert!(
            (quad[0][0] - (-1.0)).abs() < 1e-5,
            "left edge should land exactly on NDC -1, got {}",
            quad[0][0]
        );
        assert!(
            (quad[2][0] - 1.0).abs() < 1e-5,
            "right edge should land exactly on NDC 1, got {}",
            quad[2][0]
        );
        assert!(
            quad[0][1] > 1.0,
            "expected the top edge to overflow past NDC 1, got {}",
            quad[0][1]
        );
        assert!(
            quad[2][1] < -1.0,
            "expected the bottom edge to overflow past NDC -1, got {}",
            quad[2][1]
        );
    }
}
