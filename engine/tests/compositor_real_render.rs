use std::collections::HashMap;
use wer_engine::compositor::{render_textured_quad, GpuContext, TextureInput};
use wer_scene::we_shader_compile::{compile_pair, to_wgsl};

const VERTEX_WE_SOURCE: &str = r#"#version 100
precision highp float;

attribute vec3 a_Position;
attribute vec2 a_TexCoord;

varying vec4 v_TexCoord;

uniform mat4 g_ModelViewProjectionMatrix;

#include "common.h"

void main() {
    vec2 rotatedUv = rotateVec2(a_TexCoord, M_PI_HALF);
    v_TexCoord = vec4(rotatedUv, 0.0, 1.0);
    gl_Position = g_ModelViewProjectionMatrix * vec4(a_Position, 1.0);
}
"#;

const FRAGMENT_WE_SOURCE: &str = r#"#version 100
precision highp float;

uniform sampler2D g_Texture0; // {"material":"ui_editor_properties_blend_mode","combo":"BLENDMODE","type":"imageblending","default":2}
uniform sampler2D g_Texture1;
varying vec4 v_TexCoord;

#include "common.h"
#include "common_blending.h"

void main() {
    vec4 albedo = texture2D(g_Texture0, v_TexCoord.xy);
    vec4 overlay = texture2D(g_Texture1, v_TexCoord.xy);
    vec3 blended = ApplyBlending(BLENDMODE, albedo.rgb, overlay.rgb, overlay.a);
    float lum = greyscale(blended);
    gl_FragColor = vec4(mix(blended, vec3(lum), 0.0), albedo.a);
}
"#;

#[test]
fn real_we_shader_pair_renders_the_correct_multiply_blend_on_the_real_gpu() {
    let Ok(ctx) = GpuContext::new() else {
        eprintln!("no real wgpu adapter available, skipping the real-GPU render check");
        return;
    };

    let pair = compile_pair(VERTEX_WE_SOURCE, FRAGMENT_WE_SOURCE, &HashMap::new())
        .expect("the real WE shader pair should link (see we_shader_compile_pair_pipeline.rs)");
    let vertex_wgsl = to_wgsl(&pair.vertex).expect("naga WGSL codegen should succeed");
    let fragment_wgsl = to_wgsl(&pair.fragment).expect("naga WGSL codegen should succeed");

    let positions: [[f32; 3]; 6] = [
        [-1.0, -1.0, 0.0],
        [1.0, -1.0, 0.0],
        [1.0, 1.0, 0.0],
        [-1.0, -1.0, 0.0],
        [1.0, 1.0, 0.0],
        [-1.0, 1.0, 0.0],
    ];

    let tex_coords: [[f32; 2]; 6] = [[0.0, 0.0]; 6];
    let identity: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];

    let albedo: [u8; 4] = [255, 128, 64, 255];
    let overlay: [u8; 4] = [128, 255, 204, 128];

    let pixels = render_textured_quad(
        &ctx,
        &pair.vertex.module,
        &vertex_wgsl,
        &pair.fragment.module,
        &fragment_wgsl,
        &positions,
        &tex_coords,
        &identity,
        &[
            TextureInput {
                width: 1,
                height: 1,
                rgba8: &albedo,
            },
            TextureInput {
                width: 1,
                height: 1,
                rgba8: &overlay,
            },
        ],
        4,
        4,
    )
    .expect("rendering the real WE shader pair on the real GPU should succeed");

    let to_unit = |c: u8| c as f32 / 255.0;
    let a: [f32; 3] = [to_unit(albedo[0]), to_unit(albedo[1]), to_unit(albedo[2])];
    let b: [f32; 3] = [
        to_unit(overlay[0]),
        to_unit(overlay[1]),
        to_unit(overlay[2]),
    ];
    let blend = to_unit(overlay[3]);
    let expected_rgb: [f32; 3] =
        std::array::from_fn(|i| a[i] * (1.0 - blend) + a[i] * b[i] * blend);
    let expected_a = to_unit(albedo[3]);

    let idx = ((1 * 4 + 1) * 4) as usize;
    let got = &pixels[idx..idx + 4];
    let got_unit: [f32; 4] = std::array::from_fn(|i| got[i] as f32 / 255.0);

    let tol = 2.0 / 255.0;
    for i in 0..3 {
        assert!(
            (got_unit[i] - expected_rgb[i]).abs() <= tol,
            "channel {i}: expected {:.4}, got {:.4} (raw byte {}) — full pixel {:?}",
            expected_rgb[i],
            got_unit[i],
            got[i],
            pixels
        );
    }
    assert!(
        (got_unit[3] - expected_a).abs() <= tol,
        "alpha: expected {:.4}, got {:.4} (raw byte {})",
        expected_a,
        got_unit[3],
        got[3]
    );
}
