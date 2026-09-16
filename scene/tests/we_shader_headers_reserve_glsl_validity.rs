use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use wer_scene::we_shader_headers::{resolve_includes, PRELUDE_COMMON};

fn find_glslang() -> Option<String> {
    for candidate in [
        "glslangValidator",
        "/opt/homebrew/bin/glslangValidator",
        "/usr/local/bin/glslangValidator",
    ] {
        if Command::new(candidate).arg("--version").output().is_ok() {
            return Some(candidate.to_string());
        }
    }
    None
}

fn compile_or_panic(glslang: &str, source: &str) {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "wer-reserve-header-test-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("shader.frag");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();

    let output = Command::new(glslang)
        .arg(&path)
        .output()
        .expect("failed to run glslangValidator");
    if !output.status.success() {
        panic!(
            "glslangValidator rejected the assembled shader:\n{}\n{}\n--- source ---\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            source
        );
    }
}

#[test]
fn common_composite_compiles_with_glslang() {
    let Some(glslang) = find_glslang() else {
        eprintln!("glslangValidator not on PATH, skipping");
        return;
    };
    let raw = format!(
        r#"#version 100
precision highp float;

#define COMPOSITE 1
#define COMPOSITEMONO 1
#define BLENDMODE 0

{prelude}

#include "common.h"
#include "common_blending.h"
#include "common_composite.h"

uniform sampler2D g_Texture0;
varying vec4 v_TexCoord;

void main() {{
    vec4 original = texture2D(g_Texture0, v_TexCoord.xy);
    vec4 effect = texture2D(g_Texture0, v_TexCoord.zw);
    vec2 uv = ApplyCompositeOffset(v_TexCoord.xy, vec2(1.0, 1.0));
    gl_FragColor = ApplyComposite(original, effect) + vec4(uv, 0.0, 0.0);
}}
"#,
        prelude = PRELUDE_COMMON,
    );
    let resolved = resolve_includes(&raw).expect("resolve_includes should succeed");
    compile_or_panic(&glslang, &resolved);
}

#[test]
fn common_fog_compiles_with_glslang() {
    let Some(glslang) = find_glslang() else {
        eprintln!("glslangValidator not on PATH, skipping");
        return;
    };
    let raw = format!(
        r#"#version 100
precision highp float;

#define FOG_DIST 1
#define FOG_HEIGHT 1

{prelude}

#include "common.h"
#include "common_fog.h"

varying vec4 v_TexCoord;

void main() {{
    vec2 fogState = CalculateFogPixelState(v_TexCoord.x, v_TexCoord.y);
    vec3 fogged = ApplyFog(vec3(1.0, 0.5, 0.25), fogState);
    float alpha = ApplyFogAlpha(1.0, fogState);
    gl_FragColor = vec4(fogged, alpha);
}}
"#,
        prelude = PRELUDE_COMMON,
    );
    let resolved = resolve_includes(&raw).expect("resolve_includes should succeed");
    compile_or_panic(&glslang, &resolved);
}

#[test]
fn common_foliage_compiles_with_glslang() {
    let Some(glslang) = find_glslang() else {
        eprintln!("glslangValidator not on PATH, skipping");
        return;
    };
    let raw = format!(
        r#"#version 100
precision highp float;

#define LEAVESUVMODE 1

{prelude}

#include "common.h"
#include "common_foliage.h"

varying vec4 v_TexCoord;

void main() {{
    vec3 offset = CalcFoliageAnimation(
        vec3(v_TexCoord.xy, 0.0), vec3(v_TexCoord.zw, 0.0), v_TexCoord.xy, 0.5, 1.0,
        1.0, 0.5, 1.0, 0.5, 0.0, 1.0, 0.3, 5.0, 1.0, vec2(0.0, 1.0)
    );
    gl_FragColor = vec4(offset, 1.0);
}}
"#,
        prelude = PRELUDE_COMMON,
    );
    let resolved = resolve_includes(&raw).expect("resolve_includes should succeed");
    compile_or_panic(&glslang, &resolved);
}
