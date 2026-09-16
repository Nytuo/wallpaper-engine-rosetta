use std::io::Write;
use std::process::Command;
use wer_scene::we_shader_headers::{
    resolve_includes, PRELUDE_COMMON, PRELUDE_FRAGMENT_ONLY, PRELUDE_VERTEX_ONLY,
};

#[test]
fn vertex_prelude_and_headers_compile_with_glslang() {
    let Some(glslang) = find_glslang() else {
        eprintln!("glslangValidator not on PATH, skipping");
        return;
    };

    let raw = format!(
        r#"#version 100
precision highp float;

attribute vec3 a_Position;
attribute vec3 a_Normal;
attribute vec4 a_Tangent4;
attribute vec2 a_TexCoord;

varying vec4 v_TexCoord;
varying vec3 v_Normal;

uniform mat4 g_ModelViewProjectionMatrix;
uniform mat4 g_ModelMatrix;
uniform sampler2D g_Texture0;

{prelude}
{prelude_vert}

#include "common.h"
#include "common_vertex.h"

void main() {{
    mat3 modelRot = CAST3X3(g_ModelMatrix);
    mat3 tbn = BuildTangentSpace(modelRot, a_Normal, a_Tangent4);
    mat3 tbnSimple = BuildTangentSpace(a_Normal, a_Tangent4);
    vec2 rotatedUv = rotateVec2(a_TexCoord, M_PI_HALF);
    vec3 hsv = rgb2hsv(vec3(rotatedUv, 0.5));
    vec3 rgb = hsv2rgb(hsv);
    float g = greyscale(rgb);
    vec4 sampled = texSample2DLod(g_Texture0, rotatedUv, 0.0);

    v_Normal = tbn[2] + tbnSimple[2] + CAST3(g) + sampled.xyz;
    v_TexCoord = vec4(rotatedUv, 0.0, 1.0);
    gl_Position = g_ModelViewProjectionMatrix * vec4(a_Position, 1.0);
}}
"#,
        prelude = PRELUDE_COMMON,
        prelude_vert = PRELUDE_VERTEX_ONLY,
    );

    let resolved =
        resolve_includes(&raw).expect("resolve_includes should succeed for known headers");
    compile_or_panic(&glslang, &resolved, "vert");
}

#[test]
fn fragment_prelude_and_headers_compile_with_glslang() {
    let Some(glslang) = find_glslang() else {
        eprintln!("glslangValidator not on PATH, skipping");
        return;
    };

    let raw = format!(
        r#"#version 100
#extension GL_OES_standard_derivatives : enable
#extension GL_EXT_shader_texture_lod : enable
precision highp float;

uniform sampler2D g_Texture0;
uniform sampler2D g_Texture1;
varying vec4 v_TexCoord;

// A real assembled shader gets this from its own scene.json combo value
// (injected as a naga `#define`, exercised by we_shader_compile_pipeline.rs
// instead) — this test only checks the header's own syntax/types, so a
// fixed, arbitrary real value is enough.
#define BLENDMODE 0

{prelude_common}
{prelude_frag}

#include "common.h"
#include "common_fragment.h"
#include "common_blending.h"

void main() {{
    vec4 albedo = texSample2D(g_Texture0, v_TexCoord.xy);
    vec4 albedoLod = texSample2DLod(g_Texture0, v_TexCoord.xy, 0.0);
    vec4 packedNormal = texSample2D(g_Texture1, v_TexCoord.xy);

    vec3 normal = DecompressNormal(packedNormal);
    vec4 normalMasked = DecompressNormalWithMask(packedNormal);
    float r8 = ConvertSampleR8(albedo);
    vec4 standardized = ConvertTexture0Format(albedo);

    vec3 blended = ApplyBlending(0, albedo.rgb, standardized.rgb, saturate(r8));
    blended = mix(blended, normal, frac(albedoLod.a));

    float angle = atan2(v_TexCoord.y, v_TexCoord.x);
    vec2 rotated = rotateVec2(v_TexCoord.xy, angle + M_PI);
    vec2 deriv = ddx(rotated) + ddy(rotated);
    mat2 identity2 = mat2(1.0, 0.0, 0.0, 1.0);
    vec2 mulResult = mul(identity2, deriv);

    vec3 hsvColor = rgb2hsv(blended);
    vec3 rgbColor = hsv2rgb(hsvColor);
    float lum = greyscale(rgbColor);

    gl_FragColor = CAST4(lum) * vec4(rgbColor + normalMasked.rgb + mulResult.x, 1.0);
}}
"#,
        prelude_common = PRELUDE_COMMON,
        prelude_frag = PRELUDE_FRAGMENT_ONLY,
    );

    let resolved =
        resolve_includes(&raw).expect("resolve_includes should succeed for known headers");
    compile_or_panic(&glslang, &resolved, "frag");
}

fn find_glslang() -> Option<String> {
    for candidate in [
        "glslangValidator",
        "/opt/homebrew/bin/glslangValidator",
        "/usr/local/bin/glslangValidator",
    ] {
        if Command::new(candidate)
            .arg("--version")
            .output()
            .map(|o| o.status.success() || !o.stdout.is_empty())
            .unwrap_or(false)
        {
            return Some(candidate.to_string());
        }
    }
    None
}

fn compile_or_panic(glslang: &str, source: &str, stage: &str) {
    let dir = std::env::temp_dir().join(format!("wer-shader-header-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ext = match stage {
        "vert" => "vert",
        "frag" => "frag",
        _ => unreachable!(),
    };
    let path = dir.join(format!("shader.{ext}"));
    std::fs::File::create(&path)
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();

    let output = Command::new(glslang)
        .arg(&path)
        .output()
        .expect("failed to run glslangValidator");

    std::fs::remove_dir_all(&dir).ok();

    if !output.status.success() {
        panic!(
            "glslangValidator rejected the assembled {stage} shader:\n{}\n{}\n--- source ---\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            source,
        );
    }
}
