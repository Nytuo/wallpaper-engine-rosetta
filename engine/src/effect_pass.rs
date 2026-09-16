use naga::{Module, TypeInner};
use std::collections::HashMap;
use std::num::NonZeroU64;
use wer_scene::model::{ConstantValue, EffectPass as EffectPassData};
use wer_scene::we_shader_compile::{self, compile_pair, to_wgsl, PropertyDecl};

#[derive(Debug, thiserror::Error)]
pub enum EffectPassError {
    #[error("shader compile failed: {0}")]
    Compile(#[from] we_shader_compile::ShaderCompileError),
    #[error("WGSL codegen failed: {0}")]
    Wgsl(String),
    #[error("unsupported uniform shape for \"{uniform_name}\": {reason}")]
    UnsupportedUniform {
        uniform_name: String,
        reason: String,
    },
    #[error("this pass needs {needed} texture(s) (its own g_Texture0 plus any masks) but {given} were supplied")]
    WrongTextureCount { needed: usize, given: usize },
}

struct UniformBinding {
    binding: u32,
    uniform_name: String,
    material_key: Option<String>,
    size_bytes: u64,
}

pub struct CompiledEffectPass {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_bindings: Vec<UniformBinding>,

    texture_bindings: Vec<u32>,
    vertex_attributes: Vec<(u32, wgpu::VertexFormat)>,
}

impl CompiledEffectPass {
    pub fn texture_count(&self) -> usize {
        self.texture_bindings.len()
    }
}

fn infer_texture_combo_overrides(pass_data: &EffectPassData) -> HashMap<String, String> {
    let mut overrides = pass_data.combo_overrides.clone();
    let combos = we_shader_compile::parse_combos(&pass_data.vert_source)
        .into_iter()
        .chain(we_shader_compile::parse_combos(&pass_data.frag_source));
    for combo in combos {
        let Some(uniform_name) = &combo.uniform_name else {
            continue;
        };
        let Some(slot) = uniform_name
            .strip_prefix("g_Texture")
            .and_then(|n| n.parse::<usize>().ok())
        else {
            continue;
        };
        if overrides.contains_key(&combo.combo_name) {
            continue;
        }
        if pass_data
            .textures
            .get(slot)
            .and_then(|t| t.as_ref())
            .is_some()
        {
            overrides.insert(combo.combo_name, "1".to_string());
        }
    }
    overrides
}

pub fn build_effect_pass(
    device: &wgpu::Device,
    pass_data: &EffectPassData,
    output_format: wgpu::TextureFormat,
) -> Result<CompiledEffectPass, EffectPassError> {
    let combo_overrides = infer_texture_combo_overrides(pass_data);
    let pair = compile_pair(
        &pass_data.vert_source,
        &pass_data.frag_source,
        &combo_overrides,
    )?;
    let vertex_wgsl = to_wgsl(&pair.vertex).map_err(|e| EffectPassError::Wgsl(e.to_string()))?;
    let fragment_wgsl =
        to_wgsl(&pair.fragment).map_err(|e| EffectPassError::Wgsl(e.to_string()))?;

    let mut layout_entries = Vec::new();
    let mut uniform_bindings = Vec::new();
    let mut texture_bindings = Vec::new();

    for (module, properties, visibility) in [
        (
            &pair.vertex.module,
            &pair.vertex_properties,
            wgpu::ShaderStages::VERTEX,
        ),
        (
            &pair.fragment.module,
            &pair.fragment_properties,
            wgpu::ShaderStages::FRAGMENT,
        ),
    ] {
        reflect_pass_globals(
            module,
            properties,
            visibility,
            &mut layout_entries,
            &mut uniform_bindings,
            &mut texture_bindings,
        )?;
    }

    texture_bindings.sort_unstable();

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("effect-pass-bind-group-layout"),
        entries: &layout_entries,
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("effect-pass-pipeline-layout"),
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });

    let vertex_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("effect-pass-vertex"),
        source: wgpu::ShaderSource::Wgsl(vertex_wgsl.into()),
    });
    let fragment_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("effect-pass-fragment"),
        source: wgpu::ShaderSource::Wgsl(fragment_wgsl.into()),
    });

    let attributes =
        crate::compositor::reflect_vertex_attributes(&pair.vertex.module).map_err(|e| {
            EffectPassError::UnsupportedUniform {
                uniform_name: "<vertex attributes>".into(),
                reason: e.to_string(),
            }
        })?;
    let vertex_attributes: Vec<(u32, wgpu::VertexFormat)> =
        attributes.iter().map(|a| (a.location, a.format)).collect();
    let stride: u64 = vertex_attributes
        .iter()
        .map(|(_, f)| crate::compositor::vertex_format_size(*f))
        .sum();
    let mut wgpu_attrs = Vec::new();
    let mut offset = 0u64;
    for (location, format) in &vertex_attributes {
        wgpu_attrs.push(wgpu::VertexAttribute {
            format: *format,
            offset,
            shader_location: *location,
        });
        offset += crate::compositor::vertex_format_size(*format);
    }

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("effect-pass-pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &vertex_shader,
            entry_point: entry_point_name(&pair.vertex.module, naga::ShaderStage::Vertex),
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
            entry_point: entry_point_name(&pair.fragment.module, naga::ShaderStage::Fragment),
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

    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        ..Default::default()
    });

    Ok(CompiledEffectPass {
        pipeline,
        bind_group_layout,
        sampler,
        uniform_bindings,
        texture_bindings,
        vertex_attributes,
    })
}

fn reflect_pass_globals(
    module: &Module,
    properties: &[PropertyDecl],
    visibility: wgpu::ShaderStages,
    layout_entries: &mut Vec<wgpu::BindGroupLayoutEntry>,
    uniform_bindings: &mut Vec<UniformBinding>,
    texture_bindings: &mut Vec<u32>,
) -> Result<(), EffectPassError> {
    for (_, var) in module.global_variables.iter() {
        let Some(binding) = &var.binding else {
            continue;
        };
        let ty = &module.types[var.ty];
        match &ty.inner {
            TypeInner::Struct { members, span } => {
                let uniform_name = members
                    .first()
                    .and_then(|m| m.name.clone())
                    .unwrap_or_default();
                let material_key = properties
                    .iter()
                    .find(|p| p.uniform_name == uniform_name)
                    .map(|p| p.material_key.clone());
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: binding.binding,
                    visibility,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(*span as u64),
                    },
                    count: None,
                });
                uniform_bindings.push(UniformBinding {
                    binding: binding.binding,
                    uniform_name,
                    material_key,
                    size_bytes: *span as u64,
                });
            }
            TypeInner::Image { .. } => {
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: binding.binding,
                    visibility,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                });
                texture_bindings.push(binding.binding);
            }
            TypeInner::Sampler { .. } => {
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: binding.binding,
                    visibility,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                });
            }
            other => {
                return Err(EffectPassError::UnsupportedUniform {
                    uniform_name: format!("binding {}", binding.binding),
                    reason: format!("unsupported global type {other:?}"),
                })
            }
        }
    }
    Ok(())
}

const IDENTITY_MAT4: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

fn well_known_uniform_value(
    uniform_name: &str,
    texture_size: (u32, u32),
    time_seconds: f32,
) -> Option<Vec<f32>> {
    if uniform_name == "g_ModelViewProjectionMatrix"
        || uniform_name.starts_with("g_EffectModelMatrix")
        || uniform_name.starts_with("g_EffectModelViewProjectionMatrix")
        || uniform_name.starts_with("g_EffectTextureProjectionMatrix")
        || uniform_name == "g_LayerModelMatrix"
    {
        return Some(IDENTITY_MAT4.to_vec());
    }
    if uniform_name == "g_Time" {
        return Some(vec![time_seconds]);
    }
    if let Some(rest) = uniform_name
        .strip_prefix("g_Texture")
        .and_then(|s| s.strip_suffix("Resolution"))
    {
        if rest.chars().all(|c| c.is_ascii_digit()) {
            let (w, h) = texture_size;
            return Some(vec![w as f32, h as f32, w as f32, h as f32]);
        }
    }

    if uniform_name == "g_ParallaxPosition"
        || uniform_name == "g_PointerPosition"
        || uniform_name == "g_PointerPositionLast"
    {
        return Some(vec![0.5, 0.5]);
    }
    if uniform_name == "g_Screen" {
        let (w, h) = texture_size;
        let aspect = if h > 0 { w as f32 / h as f32 } else { 1.0 };
        return Some(vec![w as f32, h as f32, aspect]);
    }
    None
}

fn constant_value_components(value: &ConstantValue) -> Vec<f32> {
    match value {
        ConstantValue::Scalar(v) => vec![*v],
        ConstantValue::Vector(v) => v.clone(),
    }
}

pub struct BoundTexture<'a> {
    pub view: &'a wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

#[allow(clippy::too_many_arguments)]
pub fn render_effect_pass(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    compiled: &CompiledEffectPass,
    pass_data: &EffectPassData,
    textures: &[BoundTexture],
    target_view: &wgpu::TextureView,
    time_seconds: f32,
) -> Result<(), EffectPassError> {
    if textures.len() != compiled.texture_bindings.len() {
        return Err(EffectPassError::WrongTextureCount {
            needed: compiled.texture_bindings.len(),
            given: textures.len(),
        });
    }

    let mut owned_buffers = Vec::new();
    let mut bind_entries = Vec::new();

    for ub in &compiled.uniform_bindings {
        let components = if let Some(key) = &ub.material_key {
            pass_data.constants.get(key).map(constant_value_components)
        } else {
            None
        }
        .or_else(|| {
            well_known_uniform_value(
                &ub.uniform_name,
                (textures[0].width, textures[0].height),
                time_seconds,
            )
        })
        .unwrap_or_default();

        let mut bytes = vec![0u8; ub.size_bytes as usize];
        let component_bytes: Vec<u8> = components.iter().flat_map(|f| f.to_le_bytes()).collect();
        let copy_len = component_bytes.len().min(bytes.len());
        bytes[..copy_len].copy_from_slice(&component_bytes[..copy_len]);

        let buffer = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("effect-pass-uniform"),
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        owned_buffers.push((ub.binding, buffer));
    }
    for (binding, buffer) in &owned_buffers {
        bind_entries.push(wgpu::BindGroupEntry {
            binding: *binding,
            resource: buffer.as_entire_binding(),
        });
    }
    for (i, &binding) in compiled.texture_bindings.iter().enumerate() {
        bind_entries.push(wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(textures[i].view),
        });

        bind_entries.push(wgpu::BindGroupEntry {
            binding: binding + 1,
            resource: wgpu::BindingResource::Sampler(&compiled.sampler),
        });
    }

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("effect-pass-bind-group"),
        layout: &compiled.bind_group_layout,
        entries: &bind_entries,
    });

    let positions: [[f32; 3]; 6] = [
        [-1.0, -1.0, 0.0],
        [1.0, -1.0, 0.0],
        [1.0, 1.0, 0.0],
        [-1.0, -1.0, 0.0],
        [1.0, 1.0, 0.0],
        [-1.0, 1.0, 0.0],
    ];
    let tex_coords: [[f32; 2]; 6] = [
        [0.0, 1.0],
        [1.0, 1.0],
        [1.0, 0.0],
        [0.0, 1.0],
        [1.0, 0.0],
        [0.0, 0.0],
    ];
    let mut vertex_bytes = Vec::new();
    for i in 0..6 {
        for (_, format) in &compiled.vertex_attributes {
            match format {
                wgpu::VertexFormat::Float32x3 => {
                    vertex_bytes.extend_from_slice(bytemuck::cast_slice(&positions[i]))
                }
                wgpu::VertexFormat::Float32x2 => {
                    vertex_bytes.extend_from_slice(bytemuck::cast_slice(&tex_coords[i]))
                }
                other => {
                    return Err(EffectPassError::UnsupportedUniform {
                        uniform_name: "<vertex attribute>".into(),
                        reason: format!("unhandled attribute format {other:?}"),
                    })
                }
            }
        }
    }
    let vertex_buffer = wgpu::util::DeviceExt::create_buffer_init(
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("effect-pass-quad"),
            contents: &vertex_bytes,
            usage: wgpu::BufferUsages::VERTEX,
        },
    );

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("effect-pass-encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("effect-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target_view,
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
        pass.set_pipeline(&compiled.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.draw(0..6, 0..1);
    }
    queue.submit(Some(encoder.finish()));
    Ok(())
}

struct ChainEntry {
    prepared: Result<
        (
            CompiledEffectPass,
            Vec<(wgpu::Texture, wgpu::TextureView, u32, u32)>,
        ),
        String,
    >,
    pass_data: EffectPassData,
    index: usize,
}

pub struct CompiledEffectChain {
    entries: Vec<ChainEntry>,

    named_buffers: HashMap<String, (wgpu::Texture, wgpu::TextureView, u32, u32)>,
}

pub fn build_effect_chain(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    passes: &[EffectPassData],
    width: u32,
    height: u32,
    output_format: wgpu::TextureFormat,
) -> CompiledEffectChain {
    let mut named_buffers = HashMap::new();
    for pass in passes {
        for fbo in &pass.fbos {
            named_buffers.entry(fbo.name.clone()).or_insert_with(|| {
                let scale = fbo.scale.max(1);
                let (w, h) = ((width / scale).max(1), (height / scale).max(1));
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("effect-chain-named-fbo"),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: output_format,

                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC
                        | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                (texture, view, w, h)
            });
        }
    }

    let entries = passes
        .iter()
        .enumerate()
        .map(|(index, pass_data)| {
            let prepared = build_effect_pass(device, pass_data, output_format)
                .map_err(|e| format!("effect pass #{index} not rendered: {e}"))
                .and_then(|compiled| {
                    let needed_masks = compiled.texture_bindings.len().saturating_sub(1);
                    let mut masks = Vec::new();
                    for slot in 1..=needed_masks {
                        let asset_path = pass_data.textures.get(slot).and_then(|opt| opt.as_ref());
                        let loaded = match asset_path {
                            Some(path) => load_texture_from_png(device, queue, path)
                                .map_err(|e| format!("effect pass #{index} not rendered: {e}")),

                            None => Ok(make_placeholder_texture(device, queue)),
                        };
                        match loaded {
                            Ok(t) => masks.push(t),
                            Err(e) => return Err(e),
                        }
                    }
                    Ok((compiled, masks))
                });
            ChainEntry {
                prepared,
                pass_data: pass_data.clone(),
                index,
            }
        })
        .collect();
    CompiledEffectChain {
        entries,
        named_buffers,
    }
}

pub struct PingPongBuffers {
    buf_a: wgpu::Texture,
    view_a: wgpu::TextureView,
    buf_b: wgpu::Texture,
    view_b: wgpu::TextureView,
}

impl PingPongBuffers {
    pub fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        let make = || {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("effect-chain-ping-pong"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            (texture, view)
        };
        let (buf_a, view_a) = make();
        let (buf_b, view_b) = make();
        Self {
            buf_a,
            view_a,
            buf_b,
            view_b,
        }
    }

    pub fn result_view(&self, result_is_a: bool) -> &wgpu::TextureView {
        if result_is_a {
            &self.view_a
        } else {
            &self.view_b
        }
    }

    pub fn result_texture(&self, result_is_a: bool) -> &wgpu::Texture {
        if result_is_a {
            &self.buf_a
        } else {
            &self.buf_b
        }
    }

    pub fn into_result(self, result_is_a: bool) -> wgpu::Texture {
        if result_is_a {
            self.buf_a
        } else {
            self.buf_b
        }
    }
}

pub fn render_compiled_effect_chain(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    chain: &CompiledEffectChain,
    input: &wgpu::Texture,
    buffers: &PingPongBuffers,
    width: u32,
    height: u32,
    time_seconds: f32,
) -> (bool, Vec<String>) {
    let mut skipped = Vec::new();

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("effect-chain-seed"),
    });
    encoder.copy_texture_to_texture(
        wgpu::ImageCopyTexture {
            texture: input,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::ImageCopyTexture {
            texture: &buffers.buf_a,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let (mut current_view, mut other_view) = (&buffers.view_a, &buffers.view_b);
    let mut current_is_a = true;

    for entry in &chain.entries {
        let (compiled, masks) = match &entry.prepared {
            Ok(pair) => pair,
            Err(reason) => {
                skipped.push(reason.clone());
                continue;
            }
        };

        let mut bound = vec![BoundTexture {
            view: current_view,
            width,
            height,
        }];
        for (_, view, w, h) in masks {
            bound.push(BoundTexture {
                view,
                width: *w,
                height: *h,
            });
        }

        let mut bind_failed_on = None;
        for b in &entry.pass_data.bind {
            let resolved = if b.name == "previous" {
                Some((current_view, width, height))
            } else {
                chain
                    .named_buffers
                    .get(&b.name)
                    .map(|(_, v, w, h)| (v, *w, *h))
            };
            match resolved {
                Some((view, w, h)) => {
                    if b.index >= bound.len() {
                        bound.resize_with(b.index + 1, || BoundTexture {
                            view: current_view,
                            width,
                            height,
                        });
                    }
                    bound[b.index] = BoundTexture {
                        view,
                        width: w,
                        height: h,
                    };
                }
                None => bind_failed_on = Some(b.name.clone()),
            }
        }
        if let Some(name) = bind_failed_on {
            skipped.push(format!(
                "effect pass #{} not rendered: \"bind\" referenced unknown buffer \"{name}\"",
                entry.index
            ));
            continue;
        }

        let target_view: &wgpu::TextureView = match &entry.pass_data.target {
            Some(name) => match chain.named_buffers.get(name) {
                Some((_, v, _, _)) => v,
                None => {
                    skipped.push(format!(
                        "effect pass #{} not rendered: unknown target buffer \"{name}\"",
                        entry.index
                    ));
                    continue;
                }
            },
            None => other_view,
        };

        if let Err(e) = render_effect_pass(
            device,
            queue,
            compiled,
            &entry.pass_data,
            &bound,
            target_view,
            time_seconds,
        ) {
            skipped.push(format!("effect pass #{} not rendered: {e}", entry.index));
            continue;
        }

        if entry.pass_data.target.is_none() {
            std::mem::swap(&mut current_view, &mut other_view);
            current_is_a = !current_is_a;
        }
    }

    (current_is_a, skipped)
}

pub fn render_effect_chain(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    passes: &[EffectPassData],
    input: &wgpu::Texture,
    width: u32,
    height: u32,
    output_format: wgpu::TextureFormat,
    time_seconds: f32,
) -> Result<(wgpu::Texture, Vec<String>), EffectPassError> {
    let chain = build_effect_chain(device, queue, passes, width, height, output_format);
    let buffers = PingPongBuffers::new(device, width, height, output_format);
    let (result_is_a, skipped) = render_compiled_effect_chain(
        device,
        queue,
        &chain,
        input,
        &buffers,
        width,
        height,
        time_seconds,
    );
    Ok((buffers.into_result(result_is_a), skipped))
}

fn make_placeholder_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView, u32, u32) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("effect-chain-placeholder-texture"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
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
        &[0, 0, 0, 0],
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view, 1, 1)
}

fn load_texture_from_png(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &std::path::Path,
) -> Result<(wgpu::Texture, wgpu::TextureView, u32, u32), EffectPassError> {
    let decoded = image::open(path)
        .map_err(|e| EffectPassError::UnsupportedUniform {
            uniform_name: "<mask texture>".into(),
            reason: format!("{path:?}: {e}"),
        })?
        .into_rgba8();
    let (width, height) = decoded.dimensions();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("effect-chain-mask-texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
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
        decoded.as_raw(),
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Ok((texture, view, width, height))
}

fn entry_point_name(module: &Module, stage: naga::ShaderStage) -> &str {
    module
        .entry_points
        .iter()
        .find(|ep| ep.stage == stage)
        .map(|ep| ep.name.as_str())
        .unwrap_or("main")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_uniforms_cover_mvp_time_and_texture_resolution() {
        assert_eq!(
            well_known_uniform_value("g_ModelViewProjectionMatrix", (10, 20), 0.0)
                .unwrap()
                .len(),
            16
        );
        assert_eq!(
            well_known_uniform_value("g_Time", (10, 20), 1.5),
            Some(vec![1.5])
        );
        assert_eq!(
            well_known_uniform_value("g_Texture0Resolution", (100, 50), 0.0),
            Some(vec![100.0, 50.0, 100.0, 50.0])
        );
        assert_eq!(
            well_known_uniform_value("g_Texture1Resolution", (4, 4), 0.0),
            Some(vec![4.0, 4.0, 4.0, 4.0])
        );
        assert_eq!(
            well_known_uniform_value("g_SomeUnrelatedUniform", (4, 4), 0.0),
            None
        );
    }

    #[test]
    fn constant_value_components_flattens_scalars_and_vectors() {
        assert_eq!(
            constant_value_components(&ConstantValue::Scalar(1.5)),
            vec![1.5]
        );
        assert_eq!(
            constant_value_components(&ConstantValue::Vector(vec![0.1, 0.2, 0.3])),
            vec![0.1, 0.2, 0.3]
        );
    }
}
