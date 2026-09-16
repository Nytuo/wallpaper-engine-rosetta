use image_layers::{layer_quad_ndc, ImageLayerRenderer, LayerResources, LayerTransform};
use std::path::{Path, PathBuf};
use std::time::Instant;
use thiserror::Error;
use wer_scene::model::{LayerContent, SceneKind};
use wer_scene::Scene;

pub mod compositor;
pub mod effect_pass;
pub mod image_layers;
pub mod particles;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("no surface attached")]
    NoSurface,
    #[error("failed to acquire graphics adapter/device: {0}")]
    Adapter(String),
    #[error("failed to acquire the next surface frame: {0}")]
    Surface(String),
    #[error("failed to decode layer image {path}: {source}")]
    Image {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
    #[error("renderer for this scene layer type is not yet implemented")]
    Unimplemented,
}

pub struct Engine {
    instance: wgpu::Instance,
    gpu: Option<Gpu>,
    scene: Option<Scene>,
    scene_dir: Option<PathBuf>,

    occluded: bool,

    parallax_mouse_norm: (f32, f32),

    effect_speed: f32,
    layers: Vec<LayerState>,

    particle_layers: Vec<ParticleLayerState>,

    scene_loaded_at: Option<Instant>,

    last_tick_at: Option<Instant>,
}

struct ParticleLayerState {
    sim: particles::ParticleSim,
    renderer: particles::ParticleRenderer,
    system: wer_scene::model::ParticleSystem,
}

struct LayerState {
    resources: LayerResources,
    transform: LayerTransform,

    parallax_depth: (f32, f32),

    animated: Option<AnimatedLayer>,
}

struct AnimatedLayer {
    base_texture: wgpu::Texture,
    chain: effect_pass::CompiledEffectChain,

    buffers: effect_pass::PingPongBuffers,
    width: u32,
    height: u32,
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    renderer: ImageLayerRenderer,
}

const MAX_PARALLAX_SHIFT_FRACTION_OF_CANVAS_WIDTH: f32 = 0.05;

impl Engine {
    pub fn new() -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        Self {
            instance,
            gpu: None,
            scene: None,
            scene_dir: None,
            occluded: false,
            parallax_mouse_norm: (0.0, 0.0),
            effect_speed: 1.0,
            layers: Vec::new(),
            particle_layers: Vec::new(),
            scene_loaded_at: None,
            last_tick_at: None,
        }
    }

    pub unsafe fn attach_metal_layer(
        &mut self,
        layer: *mut std::ffi::c_void,
        width: u32,
        height: u32,
    ) -> Result<(), EngineError> {
        let surface = unsafe {
            self.instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer))
                .map_err(|e| EngineError::Adapter(e.to_string()))?
        };
        let adapter =
            pollster::block_on(self.instance.request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            }))
            .ok_or_else(|| {
                EngineError::Adapter("no real GPU adapter available for this surface".into())
            })?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
                .map_err(|e| EngineError::Adapter(e.to_string()))?;

        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .copied()
            .unwrap_or(capabilities.formats[0]);
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: capabilities.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        let renderer = ImageLayerRenderer::new(&device, format);
        self.gpu = Some(Gpu {
            device,
            queue,
            surface,
            surface_config,
            renderer,
        });
        self.rebuild_layers()?;
        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), EngineError> {
        let Some(gpu) = self.gpu.as_mut() else {
            return Err(EngineError::NoSurface);
        };
        gpu.surface_config.width = width.max(1);
        gpu.surface_config.height = height.max(1);
        gpu.surface.configure(&gpu.device, &gpu.surface_config);
        Ok(())
    }

    pub fn set_parallax_mouse_position(&mut self, x_norm: f32, y_norm: f32) {
        self.parallax_mouse_norm = (x_norm, y_norm);
    }

    pub fn set_effect_speed(&mut self, speed: f32) {
        self.effect_speed = speed;
    }

    pub fn load_scene(
        &mut self,
        scene: Scene,
        scene_dir: impl AsRef<Path>,
    ) -> Result<(), EngineError> {
        self.scene = Some(scene);
        self.scene_dir = Some(scene_dir.as_ref().to_path_buf());
        if self.gpu.is_some() {
            self.rebuild_layers()?;
        }
        Ok(())
    }

    fn rebuild_layers(&mut self) -> Result<(), EngineError> {
        self.layers.clear();
        self.particle_layers.clear();
        self.scene_loaded_at = Some(Instant::now());
        self.last_tick_at = None;
        let (Some(scene), Some(scene_dir), Some(gpu)) =
            (&self.scene, &self.scene_dir, self.gpu.as_ref())
        else {
            return Ok(());
        };
        let SceneKind::Layered { layers, .. } = &scene.kind else {
            return Ok(());
        };

        for layer in layers {
            if let LayerContent::Particles(system) = &layer.content {
                let Some(texture_path) = &system.texture else {
                    continue;
                };
                let Ok(decoded) = image::open(texture_path).map(|img| img.into_rgba8()) else {
                    continue;
                };
                let (tw, th) = decoded.dimensions();

                let renderer = particles::ParticleRenderer::new(
                    &gpu.device,
                    &gpu.queue,
                    gpu.surface_config.format,
                    &decoded.into_raw(),
                    (tw, th),
                );
                self.particle_layers.push(ParticleLayerState {
                    sim: particles::ParticleSim::new(system),
                    renderer,
                    system: system.clone(),
                });
                continue;
            }
            let LayerContent::Image { asset } = &layer.content else {
                continue;
            };
            let path = if asset.is_absolute() {
                asset.clone()
            } else {
                scene_dir.join(asset)
            };
            let decoded = image::open(&path)
                .map_err(|source| EngineError::Image {
                    path: path.clone(),
                    source,
                })?
                .into_rgba8();
            let (w, h) = decoded.dimensions();

            let transform = LayerTransform {
                x: layer.transform.x,
                y: layer.transform.y,
                width: layer.transform.width,
                height: layer.transform.height,
                rotation: layer.transform.rotation,
            };

            let renderable_passes: Vec<_> = layer.we_effect_passes.clone();

            if renderable_passes.is_empty() {
                let resources =
                    gpu.renderer
                        .upload_layer(&gpu.device, &gpu.queue, w, h, &decoded.into_raw());
                self.layers.push(LayerState {
                    resources,
                    transform,
                    parallax_depth: (layer.parallax_depth, layer.parallax_depth_y),
                    animated: None,
                });
                continue;
            }

            let base_texture = upload_rgba_texture(&gpu.device, &gpu.queue, &decoded);
            let chain = effect_pass::build_effect_chain(
                &gpu.device,
                &gpu.queue,
                &renderable_passes,
                w,
                h,
                wgpu::TextureFormat::Rgba8Unorm,
            );
            let buffers = effect_pass::PingPongBuffers::new(
                &gpu.device,
                w,
                h,
                wgpu::TextureFormat::Rgba8Unorm,
            );

            let (result_is_a, skipped) = effect_pass::render_compiled_effect_chain(
                &gpu.device,
                &gpu.queue,
                &chain,
                &base_texture,
                &buffers,
                w,
                h,
                0.0,
            );
            for reason in &skipped {
                tracing::warn!("real effect chain: {reason}");
            }
            let bind_group =
                gpu.renderer
                    .rebind_texture(&gpu.device, buffers.result_view(result_is_a), 1.0);
            let resources =
                gpu.renderer
                    .finish_layer_resources(&gpu.device, bind_group, (w as f32, h as f32));
            self.layers.push(LayerState {
                resources,
                transform,
                parallax_depth: (layer.parallax_depth, layer.parallax_depth_y),
                animated: Some(AnimatedLayer {
                    base_texture,
                    chain,
                    buffers,
                    width: w,
                    height: h,
                }),
            });
        }
        Ok(())
    }

    fn animate_layers(&mut self) {
        let Some(gpu) = self.gpu.as_ref() else { return };
        let Some(start) = self.scene_loaded_at else {
            return;
        };
        let elapsed = start.elapsed().as_secs_f32() * self.effect_speed;
        for layer in &mut self.layers {
            let Some(anim) = &layer.animated else {
                continue;
            };
            let (result_is_a, _skipped) = effect_pass::render_compiled_effect_chain(
                &gpu.device,
                &gpu.queue,
                &anim.chain,
                &anim.base_texture,
                &anim.buffers,
                anim.width,
                anim.height,
                elapsed,
            );

            layer.resources.bind_group = gpu.renderer.rebind_texture(
                &gpu.device,
                anim.buffers.result_view(result_is_a),
                1.0,
            );
        }

        let now = Instant::now();
        let dt = self
            .last_tick_at
            .map(|prev| (now - prev).as_secs_f32())
            .unwrap_or(0.0)
            .min(0.25);
        self.last_tick_at = Some(now);
        for pl in &mut self.particle_layers {
            pl.sim.step(dt * self.effect_speed, &pl.system);
        }
    }

    pub fn needs_animation(&self) -> bool {
        self.layers.iter().any(|l| l.animated.is_some()) || !self.particle_layers.is_empty()
    }

    pub fn set_occluded(&mut self, occluded: bool) {
        self.occluded = occluded;
    }

    pub fn is_occluded(&self) -> bool {
        self.occluded
    }

    pub fn tick(&mut self) -> Result<(), EngineError> {
        self.render_frame(None)
    }

    pub fn tick_and_read_pixel(&mut self, x: u32, y: u32) -> Result<[u8; 4], EngineError> {
        let mut pixel = [0u8; 4];
        self.render_frame(Some((x, y, &mut pixel)))?;
        Ok(pixel)
    }

    fn render_frame(
        &mut self,
        mut readback: Option<(u32, u32, &mut [u8; 4])>,
    ) -> Result<(), EngineError> {
        if self.occluded {
            return Ok(());
        }

        self.animate_layers();
        let Some(gpu) = self.gpu.as_ref() else {
            return Err(EngineError::NoSurface);
        };
        let Some(scene) = &self.scene else {
            return Ok(());
        };
        let SceneKind::Layered {
            canvas_width,
            canvas_height,
            parallax,
            ..
        } = &scene.kind
        else {
            return Ok(());
        };
        if self.layers.is_empty() && self.particle_layers.is_empty() {
            return Ok(());
        }

        let canvas_size = (*canvas_width, *canvas_height);
        let surface_size = (
            gpu.surface_config.width as f32,
            gpu.surface_config.height as f32,
        );
        let max_shift = canvas_width * MAX_PARALLAX_SHIFT_FRACTION_OF_CANVAS_WIDTH;
        let (mouse_x, mouse_y) = self.parallax_mouse_norm;

        for layer in &self.layers {
            let (parallax_scale_x, parallax_scale_y) = if parallax.enabled {
                let common = parallax.amount * parallax.mouse_influence * max_shift;
                (
                    common * layer.parallax_depth.0,
                    common * layer.parallax_depth.1,
                )
            } else {
                (0.0, 0.0)
            };
            let parallax_offset = (mouse_x * parallax_scale_x, mouse_y * parallax_scale_y);
            let quad = layer_quad_ndc(
                canvas_size,
                surface_size,
                &layer.transform,
                layer.resources.asset_size,
                parallax_offset,
            );
            gpu.renderer
                .update_layer_vertices(&gpu.queue, &layer.resources, &quad);
        }

        let frame = gpu
            .surface
            .get_current_texture()
            .map_err(|e| EngineError::Surface(e.to_string()))?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("wer-engine-frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("wer-engine-layered-scene-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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
            let resources: Vec<&LayerResources> =
                self.layers.iter().map(|l| &l.resources).collect();
            gpu.renderer.draw_refs(&mut pass, &resources);

            for pl in &mut self.particle_layers {
                let instances = pl.sim.instances(&pl.system);
                pl.renderer.render(
                    &gpu.device,
                    &gpu.queue,
                    &mut pass,
                    &instances,
                    canvas_size,
                    surface_size,
                );
            }
        }

        let readback_buffer = if let Some((x, y, _)) = &readback {
            let bytes_per_row = 256u32;
            let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("wer-engine-debug-pixel-readback"),
                size: bytes_per_row as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::ImageCopyTexture {
                    texture: &frame.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: *x, y: *y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::ImageCopyBuffer {
                    buffer: &buffer,
                    layout: wgpu::ImageDataLayout {
                        offset: 0,
                        bytes_per_row: Some(bytes_per_row),
                        rows_per_image: Some(1),
                    },
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            Some(buffer)
        } else {
            None
        };

        gpu.queue.submit(Some(encoder.finish()));

        if let (Some(buffer), Some((_, _, out))) = (&readback_buffer, &mut readback) {
            let slice = buffer.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
            gpu.device.poll(wgpu::Maintain::Wait);
            rx.recv()
                .map_err(|_| EngineError::Surface("debug pixel readback channel closed".into()))?
                .map_err(|e| EngineError::Surface(e.to_string()))?;
            let data = slice.get_mapped_range();
            out.copy_from_slice(&data[..4]);
            drop(data);
            buffer.unmap();
        }

        frame.present();
        Ok(())
    }
}

fn upload_rgba_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &image::RgbaImage,
) -> wgpu::Texture {
    let (width, height) = image.dimensions();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("wer-engine-effect-chain-source"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        image.as_raw(),
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
    texture
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occluded_tick_is_a_cheap_noop() {
        let mut engine = Engine::new();
        engine.set_occluded(true);
        assert!(engine.tick().is_ok());
    }

    #[test]
    fn tick_without_a_surface_reports_no_surface() {
        let mut engine = Engine::new();
        assert!(matches!(engine.tick(), Err(EngineError::NoSurface)));
    }
}
