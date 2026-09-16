use wer_engine::compositor::GpuContext;
use wer_engine::image_layers::{layer_quad_ndc, ImageLayerRenderer, LayerTransform};

#[test]
fn two_overlapping_layers_composite_with_the_correct_over_alpha_blend_on_the_real_gpu() {
    let Ok(ctx) = GpuContext::new() else {
        eprintln!("no real wgpu adapter available, skipping the real-GPU render check");
        return;
    };
    let device = &ctx.device;
    let queue = &ctx.queue;

    let output_format = wgpu::TextureFormat::Rgba8Unorm;
    let renderer = ImageLayerRenderer::new(device, output_format);

    let layer_a_rgba = [255u8, 0, 0, 255];
    let layer_a = renderer.upload_layer(device, queue, 1, 1, &layer_a_rgba);

    let layer_b_rgba = [0u8, 0, 255, 128];
    let layer_b = renderer.upload_layer(device, queue, 1, 1, &layer_b_rgba);

    let canvas = (4.0, 4.0);
    let surface = (4.0, 4.0);
    let full_canvas = LayerTransform {
        x: 2.0,
        y: 2.0,
        width: 4.0,
        height: 4.0,
        rotation: 0.0,
    };
    let quad_a = layer_quad_ndc(canvas, surface, &full_canvas, (1.0, 1.0), (0.0, 0.0));
    let quad_b = layer_quad_ndc(canvas, surface, &full_canvas, (1.0, 1.0), (0.0, 0.0));
    renderer.update_layer_vertices(queue, &layer_a, &quad_a);
    renderer.update_layer_vertices(queue, &layer_b, &quad_b);

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test-offscreen-target"),
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
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
        label: Some("test-encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("test-pass"),
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

        renderer.draw(&mut pass, &[layer_a, layer_b]);
    }

    let bytes_per_row = 256u32;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test-readback"),
        size: (bytes_per_row as u64) * 4,
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
                rows_per_image: Some(4),
            },
        },
        wgpu::Extent3d {
            width: 4,
            height: 4,
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
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();

    let row_start = bytes_per_row as usize;
    let pixel = &data[row_start..row_start + 4];

    let src_a = 128.0 / 255.0;
    let expected_r = 0.0 * src_a + 1.0 * (1.0 - src_a);
    let expected_g = 0.0;
    let expected_b = 1.0 * src_a + 0.0 * (1.0 - src_a);
    let expected_a = src_a + 1.0 * (1.0 - src_a);

    let got: [f32; 4] = std::array::from_fn(|i| pixel[i] as f32 / 255.0);
    let tol = 2.0 / 255.0;
    let expected = [expected_r, expected_g, expected_b, expected_a];
    for i in 0..4 {
        assert!(
            (got[i] - expected[i]).abs() <= tol,
            "channel {i}: expected {:.4}, got {:.4} (raw byte {}) — full pixel {:?}",
            expected[i],
            got[i],
            pixel[i],
            pixel
        );
    }

    drop(data);
    readback.unmap();
}
