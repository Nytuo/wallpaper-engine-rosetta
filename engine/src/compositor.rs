use naga::{Binding, Module, ScalarKind, TypeInner, VectorSize};
use std::num::NonZeroU64;

#[derive(Debug, thiserror::Error)]
pub enum CompositorError {
    #[error("failed to acquire a real wgpu adapter/device: {0}")]
    Adapter(String),
    #[error("shader reflection failed: {0}")]
    Reflection(String),
    #[error("naga WGSL codegen failed: {0}")]
    Wgsl(String),
    #[error("not enough textures supplied for this shader's reflected texture bindings")]
    NotEnoughTextures,
}

pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl GpuContext {
    pub fn new() -> Result<Self, CompositorError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .ok_or_else(|| CompositorError::Adapter("no real GPU adapter available".into()))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
                .map_err(|e| CompositorError::Adapter(e.to_string()))?;
        Ok(Self { device, queue })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReflectedKind {
    UniformBuffer { size_bytes: u64 },
    Texture,
    Sampler,
}

#[derive(Debug, Clone)]
struct ReflectedBinding {
    binding: u32,
    kind: ReflectedKind,
    visibility: wgpu::ShaderStages,
}

fn reflect_globals(
    module: &Module,
    visibility: wgpu::ShaderStages,
) -> Result<Vec<ReflectedBinding>, CompositorError> {
    let mut out = Vec::new();
    for (_, var) in module.global_variables.iter() {
        let Some(binding) = &var.binding else {
            continue;
        };
        let ty = &module.types[var.ty];
        let kind = match &ty.inner {
            TypeInner::Image { .. } => ReflectedKind::Texture,
            TypeInner::Sampler { .. } => ReflectedKind::Sampler,
            TypeInner::Struct { span, .. } => ReflectedKind::UniformBuffer { size_bytes: *span as u64 },
            other => {
                return Err(CompositorError::Reflection(format!(
                    "unsupported uniform type for binding {}: {other:?} (this reflector only understands the shapes \
                     we_shader_compile's transpile produces: an image, a sampler, or a uniform block wrapping one \
                     value)",
                    binding.binding
                )))
            }
        };
        out.push(ReflectedBinding {
            binding: binding.binding,
            kind,
            visibility,
        });
    }
    Ok(out)
}

pub(crate) struct ReflectedAttribute {
    pub(crate) location: u32,
    pub(crate) format: wgpu::VertexFormat,
}

pub(crate) fn reflect_vertex_attributes(
    module: &Module,
) -> Result<Vec<ReflectedAttribute>, CompositorError> {
    let entry_point = module
        .entry_points
        .iter()
        .find(|ep| ep.stage == naga::ShaderStage::Vertex)
        .ok_or_else(|| CompositorError::Reflection("module has no vertex entry point".into()))?;

    entry_point
        .function
        .arguments
        .iter()
        .map(|arg| {
            let Some(Binding::Location { location, .. }) = &arg.binding else {
                return Err(CompositorError::Reflection(format!(
                    "vertex input {:?} has no location binding",
                    arg.name
                )));
            };
            let format = vertex_format_for(&module.types[arg.ty].inner).ok_or_else(|| {
                CompositorError::Reflection(format!(
                    "unsupported vertex attribute type for {:?}",
                    arg.name
                ))
            })?;
            Ok(ReflectedAttribute {
                location: *location,
                format,
            })
        })
        .collect()
}

fn vertex_format_for(inner: &TypeInner) -> Option<wgpu::VertexFormat> {
    match inner {
        TypeInner::Scalar(s) if s.kind == ScalarKind::Float => Some(wgpu::VertexFormat::Float32),
        TypeInner::Vector { size, scalar } if scalar.kind == ScalarKind::Float => {
            Some(match size {
                VectorSize::Bi => wgpu::VertexFormat::Float32x2,
                VectorSize::Tri => wgpu::VertexFormat::Float32x3,
                VectorSize::Quad => wgpu::VertexFormat::Float32x4,
            })
        }
        _ => None,
    }
}

pub(crate) fn vertex_format_size(format: wgpu::VertexFormat) -> u64 {
    match format {
        wgpu::VertexFormat::Float32 => 4,
        wgpu::VertexFormat::Float32x2 => 8,
        wgpu::VertexFormat::Float32x3 => 12,
        wgpu::VertexFormat::Float32x4 => 16,
        other => panic!("vertex_format_size: unhandled format {other:?} (only the float formats reflect_vertex_attributes can produce are supported)"),
    }
}

pub struct TextureInput<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba8: &'a [u8],
}

#[allow(clippy::too_many_arguments)]
pub fn render_textured_quad(
    ctx: &GpuContext,
    vertex_module: &Module,
    vertex_wgsl: &str,
    fragment_module: &Module,
    fragment_wgsl: &str,
    positions: &[[f32; 3]; 6],
    tex_coords: &[[f32; 2]; 6],
    mvp: &[f32; 16],
    textures: &[TextureInput],
    output_width: u32,
    output_height: u32,
) -> Result<Vec<u8>, CompositorError> {
    let device = &ctx.device;
    let queue = &ctx.queue;

    let vertex_globals = reflect_globals(vertex_module, wgpu::ShaderStages::VERTEX)?;
    let fragment_globals = reflect_globals(fragment_module, wgpu::ShaderStages::FRAGMENT)?;
    let mut globals: Vec<ReflectedBinding> =
        vertex_globals.into_iter().chain(fragment_globals).collect();
    globals.sort_by_key(|g| g.binding);

    let attributes = reflect_vertex_attributes(vertex_module)?;

    let mut layout_entries = Vec::new();
    let mut texture_inputs = textures.iter();
    let mut owned_textures = Vec::new();
    let mut bind_entries = Vec::new();

    for g in &globals {
        match g.kind {
            ReflectedKind::UniformBuffer { size_bytes } => {
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: g.binding,
                    visibility: g.visibility,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(size_bytes),
                    },
                    count: None,
                });
                let buffer = wgpu_buffer_from_mvp(device, mvp, size_bytes);
                owned_textures.push(GpuResource::Buffer(buffer));
            }
            ReflectedKind::Texture => {
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: g.binding,
                    visibility: g.visibility,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                });
                let input = texture_inputs
                    .next()
                    .ok_or(CompositorError::NotEnoughTextures)?;
                let (texture, view) = create_texture(device, queue, input);
                owned_textures.push(GpuResource::Texture(texture, view));
            }
            ReflectedKind::Sampler => {
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: g.binding,
                    visibility: g.visibility,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                });
                let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                    mag_filter: wgpu::FilterMode::Linear,
                    min_filter: wgpu::FilterMode::Linear,
                    ..Default::default()
                });
                owned_textures.push(GpuResource::Sampler(sampler));
            }
        }
    }

    for (g, resource) in globals.iter().zip(owned_textures.iter()) {
        let binding_resource = match resource {
            GpuResource::Buffer(b) => b.as_entire_binding(),
            GpuResource::Texture(_, view) => wgpu::BindingResource::TextureView(view),
            GpuResource::Sampler(s) => wgpu::BindingResource::Sampler(s),
        };
        bind_entries.push(wgpu::BindGroupEntry {
            binding: g.binding,
            resource: binding_resource,
        });
    }

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("we-shader-bind-group-layout"),
        entries: &layout_entries,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("we-shader-bind-group"),
        layout: &bind_group_layout,
        entries: &bind_entries,
    });

    let stride: u64 = attributes
        .iter()
        .map(|a| vertex_format_size(a.format))
        .sum();
    let mut wgpu_attrs = Vec::new();
    let mut offset = 0u64;
    for a in &attributes {
        wgpu_attrs.push(wgpu::VertexAttribute {
            format: a.format,
            offset,
            shader_location: a.location,
        });
        offset += vertex_format_size(a.format);
    }
    let vertex_data = interleave_vertex_data(positions, tex_coords, &attributes);
    let vertex_buffer = wgpu::util::DeviceExt::create_buffer_init(
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("quad-vertices"),
            contents: &vertex_data,
            usage: wgpu::BufferUsages::VERTEX,
        },
    );

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("we-shader-pipeline-layout"),
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });
    let vertex_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("we-vertex-shader"),
        source: wgpu::ShaderSource::Wgsl(vertex_wgsl.into()),
    });
    let fragment_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("we-fragment-shader"),
        source: wgpu::ShaderSource::Wgsl(fragment_wgsl.into()),
    });
    let output_format = wgpu::TextureFormat::Rgba8Unorm;
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("we-shader-pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &vertex_shader,
            entry_point: entry_point_name(vertex_module, naga::ShaderStage::Vertex),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: stride,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu_attrs,
            }],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &fragment_shader,
            entry_point: entry_point_name(fragment_module, naga::ShaderStage::Fragment),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: output_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
        cache: None,
    });

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("we-shader-offscreen-target"),
        size: wgpu::Extent3d {
            width: output_width,
            height: output_height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: output_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("we-shader-encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("we-shader-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.draw(0..6, 0..1);
    }

    let bytes_per_row_unaligned = output_width * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let bytes_per_row = bytes_per_row_unaligned.div_ceil(align) * align;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("we-shader-readback"),
        size: (bytes_per_row as u64) * (output_height as u64),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::ImageCopyTexture {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::ImageCopyBuffer {
            buffer: &readback,
            layout: wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(output_height),
            },
        },
        wgpu::Extent3d {
            width: output_width,
            height: output_height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    device.poll(wgpu::Maintain::Wait);
    rx.recv()
        .expect("map_async callback should fire after Maintain::Wait")
        .expect("buffer mapping should succeed");

    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((output_width * output_height * 4) as usize);
    for row in 0..output_height {
        let start = (row * bytes_per_row) as usize;
        pixels.extend_from_slice(&data[start..start + (output_width * 4) as usize]);
    }
    drop(data);
    readback.unmap();

    Ok(pixels)
}

enum GpuResource {
    Buffer(wgpu::Buffer),

    Texture(#[allow(dead_code)] wgpu::Texture, wgpu::TextureView),
    Sampler(wgpu::Sampler),
}

fn wgpu_buffer_from_mvp(device: &wgpu::Device, mvp: &[f32; 16], size_bytes: u64) -> wgpu::Buffer {
    let mut bytes: Vec<u8> = bytemuck::cast_slice(mvp).to_vec();
    bytes.resize(size_bytes as usize, 0);
    wgpu::util::DeviceExt::create_buffer_init(
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("mvp-uniform"),
            contents: &bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        },
    )
}

fn create_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    input: &TextureInput,
) -> (wgpu::Texture, wgpu::TextureView) {
    let size = wgpu::Extent3d {
        width: input.width,
        height: input.height,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("we-layer-texture"),
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
        input.rgba8,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(input.width * 4),
            rows_per_image: Some(input.height),
        },
        size,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn interleave_vertex_data(
    positions: &[[f32; 3]; 6],
    tex_coords: &[[f32; 2]; 6],
    attributes: &[ReflectedAttribute],
) -> Vec<u8> {
    let mut out = Vec::new();
    for i in 0..6 {
        for attr in attributes {
            match attr.format {
                wgpu::VertexFormat::Float32x3 => {
                    out.extend_from_slice(bytemuck::cast_slice(&positions[i]))
                }
                wgpu::VertexFormat::Float32x2 => {
                    out.extend_from_slice(bytemuck::cast_slice(&tex_coords[i]))
                }
                other => {
                    panic!("interleave_vertex_data: unhandled reflected attribute format {other:?}")
                }
            }
        }
    }
    out
}

fn entry_point_name(module: &Module, stage: naga::ShaderStage) -> &str {
    module
        .entry_points
        .iter()
        .find(|ep| ep.stage == stage)
        .map(|ep| ep.name.as_str())
        .unwrap_or("main")
}
