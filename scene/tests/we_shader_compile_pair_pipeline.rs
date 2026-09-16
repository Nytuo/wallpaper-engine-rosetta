use std::collections::HashMap;
use std::io::Write;
use std::process::Command;
use wer_scene::we_shader_compile::{compile_pair, to_msl, to_wgsl};

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

uniform sampler2D g_Texture0; // {"material":"ui_editor_properties_blend_mode","combo":"BLENDMODE","type":"imageblending","default":0}
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
fn real_we_shader_pair_links_and_both_msl_and_wgsl_are_accepted_by_real_compilers() {
    let pair = compile_pair(VERTEX_WE_SOURCE, FRAGMENT_WE_SOURCE, &HashMap::new())
        .expect("a real WE vertex/fragment pair sharing v_TexCoord should link");

    let vertex_msl =
        to_msl(&pair.vertex).expect("naga MSL codegen should succeed for the linked vertex module");
    let fragment_msl = to_msl(&pair.fragment)
        .expect("naga MSL codegen should succeed for the linked fragment module");
    let vertex_wgsl = to_wgsl(&pair.vertex)
        .expect("naga WGSL codegen should succeed for the linked vertex module");
    let fragment_wgsl = to_wgsl(&pair.fragment)
        .expect("naga WGSL codegen should succeed for the linked fragment module");

    assert!(
        vertex_wgsl.contains("@vertex"),
        "expected a WGSL vertex entry point:\n{vertex_wgsl}"
    );
    assert!(
        fragment_wgsl.contains("@fragment"),
        "expected a WGSL fragment entry point:\n{fragment_wgsl}"
    );

    naga::front::wgsl::parse_str(&vertex_wgsl)
        .expect("naga's WGSL frontend should accept its own backend's vertex output");
    naga::front::wgsl::parse_str(&fragment_wgsl)
        .expect("naga's WGSL frontend should accept its own backend's fragment output");

    if let Some(metal) = find_xcrun_metal() {
        compile_msl_or_panic(&metal, &vertex_msl);
        compile_msl_or_panic(&metal, &fragment_msl);
    } else {
        eprintln!("xcrun metal not available, skipping the real Apple-compiler check");
    }
}

fn find_xcrun_metal() -> Option<()> {
    Command::new("xcrun")
        .args(["-sdk", "macosx", "metal", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        .then_some(())
}

fn compile_msl_or_panic(_metal: &(), msl: &str) {
    let dir = std::env::temp_dir().join(format!(
        "wer-msl-pair-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("shader.metal");
    let air_path = dir.join("shader.air");
    std::fs::File::create(&src_path)
        .unwrap()
        .write_all(msl.as_bytes())
        .unwrap();

    let output = Command::new("xcrun")
        .args(["-sdk", "macosx", "metal", "-c"])
        .arg(&src_path)
        .arg("-o")
        .arg(&air_path)
        .output()
        .expect("failed to run xcrun metal");

    std::fs::remove_dir_all(&dir).ok();

    if !output.status.success() {
        panic!(
            "xcrun metal rejected the linked pair's MSL:\n{}\n{}\n--- MSL ---\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            msl,
        );
    }
}
