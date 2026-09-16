use wer_engine::compositor::GpuContext;
use wer_engine::particles::{ParticleInstance, ParticleRenderer};

fn readback(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let bytes_per_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test-readback"),
        size: (bytes_per_row as u64) * (height as u64),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::ImageCopyTexture {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::ImageCopyBuffer {
            buffer: &buffer,
            layout: wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::Maintain::Wait);
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        let start = (row * bytes_per_row) as usize;
        out.extend_from_slice(&data[start..start + (width * 4) as usize]);
    }
    out
}

#[test]
fn a_real_additively_blended_particle_actually_lightens_the_background() {
    let Ok(ctx) = GpuContext::new() else {
        eprintln!("no real wgpu adapter available, skipping");
        return;
    };
    let device = &ctx.device;
    let queue = &ctx.queue;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let (width, height) = (64u32, 64u32);

    let white_pixel = [255u8, 255, 255, 255];
    let mut renderer = ParticleRenderer::new(device, queue, format, &white_pixel, (1, 1));

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("particle-test-target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let background: f32 = 0.25;
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("particle-test-encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("particle-test-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: background as f64,
                        g: background as f64,
                        b: background as f64,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        let instances = [ParticleInstance {
            pos: [32.0, 32.0],
            size: 40.0,
            color: [1.0, 1.0, 1.0, 0.5],
        }];
        renderer.render(
            device,
            queue,
            &mut pass,
            &instances,
            (width as f32, height as f32),
            (width as f32, height as f32),
        );
    }
    queue.submit(Some(encoder.finish()));

    let pixels = readback(device, queue, &target, width, height);
    let idx = ((height / 2 * width + width / 2) * 4) as usize;
    let got = [
        pixels[idx] as f32 / 255.0,
        pixels[idx + 1] as f32 / 255.0,
        pixels[idx + 2] as f32 / 255.0,
    ];

    let expected = background + 1.0 * 0.5;
    let tol = 2.0 / 255.0;
    for (i, channel) in got.iter().enumerate() {
        assert!(
            (channel - expected).abs() < tol,
            "channel {i}: expected additive-blended {expected:.3}, got {:.3} (background alone would be {background:.3})",
            channel
        );
    }
}
