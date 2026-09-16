use std::collections::HashMap;
use std::io::Write;
use std::process::Command;
use wer_scene::we_shader_compile::{compile, to_msl, ShaderStage};

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
fn vertex_shader_compiles_to_msl_and_apple_metal_accepts_it() {
    let compiled = compile(VERTEX_WE_SOURCE, ShaderStage::Vertex, &HashMap::new())
        .expect("real WE-dialect vertex shader should compile through the full Phase C pipeline");
    let msl = to_msl(&compiled).expect("naga MSL codegen should succeed for a validated module");
    assert!(
        msl.contains("vertex"),
        "expected naga to emit a Metal vertex entry point:\n{msl}"
    );

    let Some(metal) = find_xcrun_metal() else {
        eprintln!("xcrun metal not available, skipping the real-compiler check");
        return;
    };
    compile_msl_or_panic(&metal, &msl);
}

#[test]
fn fragment_shader_with_a_real_blendmode_combo_compiles_to_msl_and_apple_metal_accepts_it() {
    let compiled = compile(FRAGMENT_WE_SOURCE, ShaderStage::Fragment, &HashMap::new())
        .expect("real WE-dialect fragment shader with a BLENDMODE combo should compile");
    let msl = to_msl(&compiled).expect("naga MSL codegen should succeed for a validated module");
    assert!(
        msl.contains("fragment"),
        "expected naga to emit a Metal fragment entry point:\n{msl}"
    );

    let Some(metal) = find_xcrun_metal() else {
        eprintln!("xcrun metal not available, skipping the real-compiler check");
        return;
    };
    compile_msl_or_panic(&metal, &msl);
}

#[test]
fn combo_override_actually_changes_the_generated_module() {
    let mut overrides_zero = HashMap::new();
    overrides_zero.insert("BLENDMODE".to_string(), "0".to_string());
    let mut overrides_one = HashMap::new();
    overrides_one.insert("BLENDMODE".to_string(), "1".to_string());

    let msl_zero =
        to_msl(&compile(FRAGMENT_WE_SOURCE, ShaderStage::Fragment, &overrides_zero).unwrap())
            .unwrap();
    let msl_one =
        to_msl(&compile(FRAGMENT_WE_SOURCE, ShaderStage::Fragment, &overrides_one).unwrap())
            .unwrap();
    assert_ne!(
        msl_zero, msl_one,
        "different BLENDMODE overrides should produce different constant-folded MSL"
    );
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
        "wer-msl-test-{}-{}",
        std::process::id(),
        fastrand_stub()
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
            "xcrun metal rejected the naga-generated MSL:\n{}\n{}\n--- MSL ---\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            msl,
        );
    }
}

fn fastrand_stub() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}
