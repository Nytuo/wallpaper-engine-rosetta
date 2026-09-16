use std::num::NonZeroU64;
use wer_scene::model::{ParticleModule, ParticleSystem};

#[derive(Debug, Clone, Copy)]
struct Particle {
    pos: [f32; 2],
    vel: [f32; 2],
    age: f32,
    lifetime: f32,
    size: f32,
    color: [f32; 3],
    alpha: f32,

    max_alpha: f32,
    fade_in_time: f32,

    osc_frequency: f32,
    osc_phase: f32,
    osc_scale: f32,
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, min: f32, max: f32) -> f32 {
        min + (max - min) * self.next_f32()
    }
}

fn find_module<'a>(modules: &'a [ParticleModule], name: &str) -> Option<&'a ParticleModule> {
    modules.iter().find(|m| m.name == name)
}

fn param_f32(m: &ParticleModule, key: &str, default: f32) -> f32 {
    m.params
        .get(key)
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .unwrap_or(default)
}

fn param_vec3(m: &ParticleModule, key: &str, default: [f32; 3]) -> [f32; 3] {
    let Some(v) = m.params.get(key) else {
        return default;
    };
    if let Some(n) = v.as_f64() {
        return [n as f32; 3];
    }
    if let Some(s) = v.as_str() {
        let mut parts = s.split_whitespace().filter_map(|p| p.parse::<f32>().ok());
        return [
            parts.next().unwrap_or(default[0]),
            parts.next().unwrap_or(default[1]),
            parts.next().unwrap_or(default[2]),
        ];
    }
    default
}

fn param_color3(m: &ParticleModule, key: &str, default: [f32; 3]) -> [f32; 3] {
    let raw = param_vec3(
        m,
        key,
        [default[0] * 255.0, default[1] * 255.0, default[2] * 255.0],
    );
    [raw[0] / 255.0, raw[1] / 255.0, raw[2] / 255.0]
}

pub struct ParticleSim {
    particles: Vec<Particle>,
    spawn_accumulator: f32,
    rng: Rng,
    max_count: usize,
}

impl ParticleSim {
    pub fn new(system: &ParticleSystem) -> Self {
        Self {
            particles: Vec::new(),
            spawn_accumulator: 0.0,
            rng: Rng::new(
                (system.maxcount as u64)
                    .wrapping_mul(0x9E3779B97F4A7C15)
                    .wrapping_add(1),
            ),
            max_count: system.maxcount as usize,
        }
    }

    pub fn step(&mut self, dt: f32, system: &ParticleSystem) {
        if dt <= 0.0 {
            return;
        }

        if let Some(emitter) = system.emitters.first() {
            let rate = param_f32(emitter, "rate", 0.0).max(0.0);
            if rate > 0.0 {
                self.spawn_accumulator += dt * rate;
                let mut to_spawn = self.spawn_accumulator.floor() as i32;
                self.spawn_accumulator -= to_spawn as f32;
                while to_spawn > 0 && self.particles.len() < self.max_count {
                    self.spawn_one(system, emitter);
                    to_spawn -= 1;
                }
            }
        }

        let movement = find_module(&system.operators, "movement");
        let alphafade = find_module(&system.operators, "alphafade");
        let gravity = movement
            .map(|m| param_vec3(m, "gravity", [0.0, 0.0, 0.0]))
            .unwrap_or([0.0, 0.0, 0.0]);
        let drag = movement.map(|m| param_f32(m, "drag", 0.0)).unwrap_or(0.0);
        let fade_in_time_default = alphafade
            .map(|m| param_f32(m, "fadeintime", 0.0))
            .unwrap_or(0.0);

        let elapsed_hint = self.particles.first().map(|p| p.age).unwrap_or(0.0);
        let _ = elapsed_hint;

        self.particles.retain_mut(|p| {
            p.age += dt;
            if p.age >= p.lifetime {
                return false;
            }

            p.vel[0] += gravity[0] * dt;
            p.vel[1] += gravity[1] * dt;
            if drag > 0.0 {
                let damping = 1.0 / (1.0 + drag * dt);
                p.vel[0] *= damping;
                p.vel[1] *= damping;
            }
            p.pos[0] += p.vel[0] * dt;
            p.pos[1] += p.vel[1] * dt;

            let fade_in_time = p.fade_in_time.max(fade_in_time_default);
            let fade_in = if fade_in_time > 0.0 {
                (p.age / fade_in_time).min(1.0)
            } else {
                1.0
            };
            let remaining = p.lifetime - p.age;
            let fade_out = if fade_in_time > 0.0 {
                (remaining / fade_in_time).min(1.0)
            } else {
                1.0
            };
            p.alpha = fade_in.min(fade_out).max(0.0) * p.max_alpha;

            true
        });
    }

    fn spawn_one(&mut self, system: &ParticleSystem, emitter: &ParticleModule) {
        let origin = param_vec3(emitter, "origin", [0.0, 0.0, 0.0]);
        let directions = param_vec3(emitter, "directions", [1.0, 1.0, 1.0]);
        let dist_min = param_f32(emitter, "distancemin", 0.0);
        let dist_max = param_f32(emitter, "distancemax", 0.0);

        let angle = self.rng.range(0.0, std::f32::consts::TAU);
        let mut dir = [angle.cos() * directions[0], angle.sin() * directions[1]];
        let len = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt().max(1e-6);
        dir = [dir[0] / len, dir[1] / len];
        let dist = self.rng.range(dist_min, dist_max);

        let local_pos = [origin[0] + dir[0] * dist, origin[1] + dir[1] * dist];
        let pos = apply_system_transform(system, local_pos);

        let lifetime = find_module(&system.initializers, "lifetimerandom")
            .map(|m| {
                self.rng
                    .range(param_f32(m, "min", 1.0), param_f32(m, "max", 1.0))
            })
            .unwrap_or(1.0)
            .max(0.01);

        let size = find_module(&system.initializers, "sizerandom")
            .map(|m| {
                let min = param_f32(m, "min", 1.0);
                let max = param_f32(m, "max", 1.0);
                let exponent = param_f32(m, "exponent", 1.0).max(0.01);

                let u = self.rng.next_f32().powf(1.0 / exponent);
                min + (max - min) * u
            })
            .unwrap_or(1.0)
            * system.scale;

        let velocity_local = find_module(&system.initializers, "velocityrandom")
            .map(|m| {
                let min = param_vec3(m, "min", [0.0, 0.0, 0.0]);
                let max = param_vec3(m, "max", [0.0, 0.0, 0.0]);
                [
                    self.rng.range(min[0], max[0]),
                    self.rng.range(min[1], max[1]),
                ]
            })
            .unwrap_or([0.0, 0.0]);
        let vel = apply_system_rotation(system, velocity_local);

        let color = find_module(&system.initializers, "colorrandom")
            .map(|m| {
                let min = param_color3(m, "min", [1.0, 1.0, 1.0]);
                let max = param_color3(m, "max", [1.0, 1.0, 1.0]);
                [
                    self.rng.range(min[0], max[0]),
                    self.rng.range(min[1], max[1]),
                    self.rng.range(min[2], max[2]),
                ]
            })
            .unwrap_or([1.0, 1.0, 1.0]);

        let alpha0 = find_module(&system.initializers, "alpharandom")
            .map(|m| {
                let min = param_f32(m, "min", 1.0);
                let max = param_f32(m, "max", 1.0);
                let exponent = param_f32(m, "exponent", 1.0).max(0.01);
                let u = self.rng.next_f32().powf(1.0 / exponent);
                min + (max - min) * u
            })
            .unwrap_or(1.0);

        let fade_in_time = find_module(&system.operators, "alphafade")
            .map(|m| param_f32(m, "fadeintime", 0.0))
            .unwrap_or(0.0);

        let osc = find_module(&system.operators, "oscillateposition");
        let osc_frequency = osc
            .map(|m| {
                self.rng.range(
                    param_f32(m, "frequencymin", 1.0),
                    param_f32(m, "frequencymax", 1.0),
                )
            })
            .unwrap_or(0.0);
        let osc_phase = osc
            .map(|m| {
                self.rng
                    .range(param_f32(m, "phasemin", 0.0), param_f32(m, "phasemax", 0.0))
            })
            .unwrap_or(0.0);
        let osc_scale = osc
            .map(|m| {
                self.rng
                    .range(param_f32(m, "scalemin", 0.0), param_f32(m, "scalemax", 0.0))
            })
            .unwrap_or(0.0);

        self.particles.push(Particle {
            pos,
            vel,
            age: 0.0,
            lifetime,
            size,
            color,
            alpha: 0.0,
            max_alpha: alpha0,
            fade_in_time,
            osc_frequency,
            osc_phase,
            osc_scale,
        });
    }

    pub fn instances(&self, system: &ParticleSystem) -> Vec<ParticleInstance> {
        let osc_mask = find_module(&system.operators, "oscillateposition")
            .map(|m| param_vec3(m, "mask", [0.0, 0.0, 0.0]));
        self.particles
            .iter()
            .map(|p| {
                let mut pos = p.pos;
                if let Some(mask) = osc_mask {
                    let s = (p.osc_phase + p.osc_frequency * p.age).sin() * p.osc_scale;
                    pos[0] += s * mask[0] * system.scale;
                    pos[1] += s * mask[1] * system.scale;
                }
                ParticleInstance {
                    pos,
                    size: p.size,
                    color: [p.color[0], p.color[1], p.color[2], p.alpha],
                }
            })
            .collect()
    }
}

fn apply_system_transform(system: &ParticleSystem, local: [f32; 2]) -> [f32; 2] {
    let scaled = [local[0] * system.scale, local[1] * system.scale];
    let rotated = apply_system_rotation(system, scaled);
    [rotated[0] + system.origin.0, rotated[1] + system.origin.1]
}

fn apply_system_rotation(system: &ParticleSystem, v: [f32; 2]) -> [f32; 2] {
    let (s, c) = system.rotation.sin_cos();
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c]
}

pub struct ParticleInstance {
    pub pos: [f32; 2],
    pub size: f32,
    pub color: [f32; 4],
}

const PARTICLE_SHADER: &str = r#"
struct CanvasUniform {
    canvas_size: vec2<f32>,
    surface_size: vec2<f32>,
}
@group(0) @binding(0) var<uniform> u_canvas: CanvasUniform;
@group(0) @binding(1) var t_particle: texture_2d<f32>;
@group(0) @binding(2) var s_particle: sampler;

struct Instance {
    @location(0) pos: vec2<f32>,
    @location(1) size: f32,
    @location(2) color: vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, instance: Instance) -> VertexOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-0.5, -0.5), vec2<f32>(0.5, -0.5), vec2<f32>(0.5, 0.5),
        vec2<f32>(-0.5, -0.5), vec2<f32>(0.5, 0.5), vec2<f32>(-0.5, 0.5),
    );
    var uvs = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 1.0), vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 0.0),
    );
    let corner = corners[vertex_index];
    let canvas_pos = instance.pos + corner * instance.size;

    let scale = max(u_canvas.surface_size.x / u_canvas.canvas_size.x, u_canvas.surface_size.y / u_canvas.canvas_size.y);
    let surface_pos = (canvas_pos - u_canvas.canvas_size * 0.5) * scale + u_canvas.surface_size * 0.5;
    let ndc = vec2<f32>(
        (surface_pos.x / u_canvas.surface_size.x) * 2.0 - 1.0,
        1.0 - (surface_pos.y / u_canvas.surface_size.y) * 2.0,
    );

    var out: VertexOut;
    out.clip_position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = uvs[vertex_index];
    out.color = instance.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let sampled = textureSample(t_particle, s_particle, in.uv);
    return sampled * in.color;
}
"#;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuInstance {
    pos: [f32; 2],
    size: f32,
    color: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CanvasUniform {
    canvas_size: [f32; 2],
    surface_size: [f32; 2],
}

pub struct ParticleRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    texture_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
}

impl ParticleRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        output_format: wgpu::TextureFormat,
        texture_rgba: &[u8],
        texture_size: (u32, u32),
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particle-shader"),
            source: wgpu::ShaderSource::Wgsl(PARTICLE_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle-bind-group-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
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
            label: Some("particle-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Float32x4],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particle-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[instance_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,

                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle-canvas-uniform"),
            size: std::mem::size_of::<CanvasUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("particle-texture"),
            size: wgpu::Extent3d {
                width: texture_size.0,
                height: texture_size.1,
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
            texture_rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(texture_size.0 * 4),
                rows_per_image: Some(texture_size.1),
            },
            wgpu::Extent3d {
                width: texture_size.0,
                height: texture_size.1,
                depth_or_array_layers: 1,
            },
        );
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("particle-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle-bind-group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let instance_capacity = 64;
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle-instances"),
            size: (instance_capacity * std::mem::size_of::<GpuInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            bind_group_layout,
            uniform_buffer,
            texture_view,
            sampler,
            bind_group,
            instance_buffer,
            instance_capacity,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render<'pass>(
        &'pass mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'pass>,
        instances: &[ParticleInstance],
        canvas_size: (f32, f32),
        surface_size: (f32, f32),
    ) {
        if instances.is_empty() {
            return;
        }
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&CanvasUniform {
                canvas_size: [canvas_size.0, canvas_size.1],
                surface_size: [surface_size.0, surface_size.1],
            }),
        );

        if instances.len() > self.instance_capacity {
            self.instance_capacity = (instances.len() * 2).max(64);
            self.instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("particle-instances"),
                size: (self.instance_capacity * std::mem::size_of::<GpuInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        let gpu_instances: Vec<GpuInstance> = instances
            .iter()
            .map(|i| GpuInstance {
                pos: i.pos,
                size: i.size,
                color: i.color,
            })
            .collect();
        queue.write_buffer(
            &self.instance_buffer,
            0,
            bytemuck::cast_slice(&gpu_instances),
        );

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        pass.draw(0..6, 0..instances.len() as u32);
    }

    #[allow(dead_code)]
    fn unused_fields_reserved_for_future_texture_swap(
        &self,
    ) -> (&wgpu::BindGroupLayout, &wgpu::TextureView, &wgpu::Sampler) {
        (&self.bind_group_layout, &self.texture_view, &self.sampler)
    }
}
