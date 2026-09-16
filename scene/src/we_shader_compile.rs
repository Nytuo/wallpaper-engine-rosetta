use crate::we_shader_headers;
use crate::we_shader_headers::{resolve_includes, ShaderHeaderError};
use naga::valid::{Capabilities, ModuleInfo, ValidationFlags, Validator};
use naga::{FastHashMap, Module};
use std::collections::HashMap;

pub use naga::ShaderStage;

#[derive(Debug, thiserror::Error)]
pub enum ShaderCompileError {
    #[error("#include resolution failed: {0}")]
    Include(#[from] ShaderHeaderError),
    #[error("failed to parse translated GLSL as naga module: {0:?}")]
    Parse(Vec<naga::front::glsl::Error>),
    #[error("translated module failed naga validation: {0}")]
    Validate(String),
    #[error("MSL codegen failed: {0}")]
    Msl(String),
    #[error("WGSL codegen failed: {0}")]
    Wgsl(String),
    #[error("fragment shader reads varying \"{0}\" that the vertex shader never writes — can't link this pair into one pipeline")]
    UnmatchedVarying(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComboDecl {
    pub uniform_name: Option<String>,
    pub combo_name: String,
    pub default: i64,
}

pub fn parse_combos(source: &str) -> Vec<ComboDecl> {
    let mut combos = Vec::new();
    for line in source.lines() {
        let Some(comment_start) = line.find("//") else {
            continue;
        };
        let (decl, comment) = line.split_at(comment_start);
        let uniform_name =
            uniform_name_from_decl(decl.trim()).filter(|_| decl.trim().starts_with("uniform "));
        let json_text = comment[2..]
            .trim()
            .strip_prefix("[COMBO]")
            .map(str::trim)
            .unwrap_or(comment[2..].trim());
        let Ok(json) = serde_json::from_str::<serde_json::Value>(json_text) else {
            continue;
        };
        let Some(combo_name) = json.get("combo").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let default = json
            .get("default")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        combos.push(ComboDecl {
            uniform_name,
            combo_name: combo_name.to_string(),
            default,
        });
    }
    combos
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyDecl {
    pub uniform_name: String,
    pub material_key: String,

    pub glsl_type: String,
}

pub fn parse_material_properties(source: &str) -> Vec<PropertyDecl> {
    let mut properties = Vec::new();
    for line in source.lines() {
        let Some(comment_start) = line.find("//") else {
            continue;
        };
        let (decl, comment) = line.split_at(comment_start);
        let decl = decl.trim();
        if !decl.starts_with("uniform ") {
            continue;
        }
        let Some(uniform_name) = uniform_name_from_decl(decl) else {
            continue;
        };
        let Some(glsl_type) = decl
            .strip_prefix("uniform ")
            .and_then(|rest| rest.split_whitespace().next())
        else {
            continue;
        };
        let json_text = comment[2..]
            .trim()
            .strip_prefix("[COMBO]")
            .map(str::trim)
            .unwrap_or(comment[2..].trim());
        let Ok(json) = serde_json::from_str::<serde_json::Value>(json_text) else {
            continue;
        };
        let Some(material_key) = json.get("material").and_then(serde_json::Value::as_str) else {
            continue;
        };
        properties.push(PropertyDecl {
            uniform_name,
            material_key: material_key.to_string(),
            glsl_type: glsl_type.to_string(),
        });
    }
    properties
}

fn uniform_name_from_decl(decl: &str) -> Option<String> {
    let decl = decl.trim_end_matches(';').trim();
    decl.rsplit(' ').next().map(|s| s.to_string())
}

fn assign_uniform_bindings(source: &str) -> String {
    let mut next_binding: u32 = 0;
    assign_uniform_bindings_from(source, &mut next_binding)
}

fn assign_uniform_bindings_from(source: &str, next_binding: &mut u32) -> String {
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some((ty, name, comment)) = parse_bare_uniform_decl(trimmed) {
            let binding = *next_binding;
            *next_binding += 1;
            if is_opaque_type(ty) {
                out.push_str(&format!(
                    "layout(binding={binding}) uniform {ty} {name};{comment}\n"
                ));
            } else {
                out.push_str(&format!(
                    "layout(binding={binding}) uniform {name}_ubo {{ {ty} {name}; }};{comment}\n"
                ));
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn parse_bare_uniform_decl(trimmed: &str) -> Option<(&str, &str, &str)> {
    let rest = trimmed.strip_prefix("uniform ")?;
    let (decl, comment) = match rest.find("//") {
        Some(i) => (&rest[..i], &rest[i.saturating_sub(1)..]),
        None => (rest, ""),
    };
    let decl = decl.trim().strip_suffix(';')?.trim();
    let mut parts = decl.split_whitespace();
    let ty = parts.next()?;
    let name = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some((ty, name, comment))
}

fn is_opaque_type(ty: &str) -> bool {
    for prefix in [
        "sampler", "isampler", "usampler", "texture", "itexture", "utexture",
    ] {
        if ty.starts_with(prefix) {
            return true;
        }
    }
    false
}

const COMBINED_SAMPLER_TYPES: &[(&str, &str)] = &[
    ("sampler2D", "texture2D"),
    ("sampler2DArray", "texture2DArray"),
    ("sampler3D", "texture3D"),
    ("samplerCube", "textureCube"),
    ("samplerCubeArray", "textureCubeArray"),
];

fn split_combined_sampler_declarations(source: &str) -> (String, HashMap<String, String>) {
    let mut names = HashMap::new();
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some((ty, name, comment)) = parse_bare_uniform_decl(trimmed) {
            if let Some((_, texture_ty)) = COMBINED_SAMPLER_TYPES.iter().find(|(t, _)| *t == ty) {
                names.insert(name.to_string(), ty.to_string());
                out.push_str(&format!("uniform {texture_ty} {name};{comment}\n"));
                out.push_str(&format!("uniform sampler {name}_sampler;\n"));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    (out, names)
}

fn rewrite_split_sampler_call_sites(source: &str, names: &HashMap<String, String>) -> String {
    let mut out = source.to_string();
    for (name, combined_ty) in names {
        for func in ["texture", "textureLod"] {
            let pattern = format!("{func}({name},");
            let replacement = format!("{func}({combined_ty}({name}, {name}_sampler),");
            out = out.replace(&pattern, &replacement);
        }
    }
    out
}

fn assign_io_locations(source: &str) -> String {
    let mut in_locations = HashMap::new();
    let mut in_next = 0;
    let mut out_locations = HashMap::new();
    let mut out_next = 0;
    let result = number_named_io(source, "in ", &mut in_locations, &mut in_next, false)
        .expect("assigning fresh locations never fails to find an existing one");
    number_named_io(&result, "out ", &mut out_locations, &mut out_next, false)
        .expect("assigning fresh locations never fails to find an existing one")
}

fn number_named_io(
    source: &str,
    qualifier: &str,
    locations: &mut HashMap<String, u32>,
    next: &mut u32,
    reuse_existing: bool,
) -> Result<String, ShaderCompileError> {
    let mut result = String::with_capacity(source.len());
    for line in source.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix(qualifier) {
            let name = io_var_name(rest).unwrap_or_default();
            let location = if reuse_existing {
                *locations
                    .get(&name)
                    .ok_or_else(|| ShaderCompileError::UnmatchedVarying(name.clone()))?
            } else {
                let loc = *next;
                *next += 1;
                locations.insert(name.clone(), loc);
                loc
            };
            result.push_str(&format!("layout(location={location}) {qualifier}{rest}\n"));
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }
    Ok(result)
}

fn io_var_name(decl: &str) -> Option<String> {
    let decl = decl.trim_end().trim_end_matches(';').trim();
    decl.rsplit(' ').next().map(|s| s.to_string())
}

pub fn transpile_dialect(source: &str, stage: ShaderStage) -> String {
    let mut body = rewrite_dialect_body(source, stage);
    body = assign_uniform_bindings(&body);

    let mut out = String::with_capacity(body.len() + 64);
    out.push_str("#version 450 core\n");
    out.push_str(&prelude_for(stage));
    if stage == ShaderStage::Fragment && body.contains("gl_FragColor") {
        out.push_str("out vec4 fragColor;\n");
        body = body.replace("gl_FragColor", "fragColor");
    }
    out.push_str(&body);
    assign_io_locations(&out)
}

fn prelude_for(stage: ShaderStage) -> String {
    const COMMON_TEX_SAMPLE_2D: &str =
        "vec4 texSample2D(sampler2D tex, vec2 uv) { return texture2D(tex, uv); }";
    const FRAGMENT_TEX_SAMPLE_2D_LOD: &str =
        "vec4 texSample2DLod(sampler2D tex, vec2 uv, float lod) { return texture2DLodEXT(tex, uv, lod); }";
    const VERTEX_TEX_SAMPLE_2D_LOD: &str =
        "vec4 texSample2DLod(sampler2D tex, vec2 uv, float lod) { return texture2DLod(tex, uv, lod); }";

    let mut prelude = we_shader_headers::PRELUDE_COMMON.replace(COMMON_TEX_SAMPLE_2D, "");
    prelude.push_str(&match stage {
        ShaderStage::Vertex => {
            we_shader_headers::PRELUDE_VERTEX_ONLY.replace(VERTEX_TEX_SAMPLE_2D_LOD, "")
        }
        ShaderStage::Fragment => {
            we_shader_headers::PRELUDE_FRAGMENT_ONLY.replace(FRAGMENT_TEX_SAMPLE_2D_LOD, "")
        }
        ShaderStage::Compute => String::new(),
    });
    prelude
}

fn rewrite_dialect_body(source: &str, stage: ShaderStage) -> String {
    let varying_replacement = match stage {
        ShaderStage::Vertex => "out ",
        ShaderStage::Fragment => "in ",
        ShaderStage::Compute => "in ",
    };

    let mut body = String::with_capacity(source.len());
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("precision ")
            || trimmed.starts_with("#extension")
            || trimmed.starts_with("#version")
        {
            continue;
        }
        let mut rewritten = line.to_string();
        if let Some(rest) = rewritten.strip_prefix("attribute ") {
            rewritten = format!("in {rest}");
        } else if let Some(rest) = rewritten.strip_prefix("varying ") {
            rewritten = format!("{varying_replacement}{rest}");
        }
        body.push_str(&rewritten);
        body.push('\n');
    }

    body = body
        .replace("texture2DLodEXT(", "textureLod(")
        .replace("texture2DLod(", "textureLod(")
        .replace("texture2D(", "texture(")
        .replace("texSample2DLod(", "textureLod(")
        .replace("texSample2D(", "texture(");

    let (split_body, split_names) = split_combined_sampler_declarations(&body);
    let body = rewrite_split_sampler_call_sites(&split_body, &split_names);
    rename_reserved_identifiers(&body)
}

const NAGA_RESERVED_IDENTIFIERS: &[(&str, &str)] = &[("sample", "we_sample_local")];

fn rename_reserved_identifiers(source: &str) -> String {
    let mut result = source.to_string();
    for (reserved, replacement) in NAGA_RESERVED_IDENTIFIERS {
        result = replace_whole_word(&result, reserved, replacement);
    }
    result
}

fn replace_whole_word(source: &str, word: &str, replacement: &str) -> String {
    fn is_ident_char(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_'
    }

    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(pos) = rest.find(word) {
        let before = &rest[..pos];
        let after = &rest[pos + word.len()..];
        let before_ok = before
            .chars()
            .next_back()
            .map_or(true, |c| !is_ident_char(c));
        let after_ok = after.chars().next().map_or(true, |c| !is_ident_char(c));
        if before_ok && after_ok {
            out.push_str(before);
            out.push_str(replacement);
        } else {
            out.push_str(before);
            out.push_str(word);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

#[derive(Debug)]
pub struct CompiledShader {
    pub module: Module,
    pub info: ModuleInfo,
}

pub fn compile(
    source: &str,
    stage: ShaderStage,
    combo_overrides: &HashMap<String, String>,
) -> Result<CompiledShader, ShaderCompileError> {
    let resolved = resolve_includes(source)?;
    let combos = parse_combos(&resolved);
    let translated = transpile_dialect(&resolved, stage);
    parse_and_validate(&translated, stage, &combos, combo_overrides)
}

fn parse_and_validate(
    translated: &str,
    stage: ShaderStage,
    combos: &[ComboDecl],
    combo_overrides: &HashMap<String, String>,
) -> Result<CompiledShader, ShaderCompileError> {
    let mut defines = FastHashMap::default();
    for combo in combos {
        let value = combo_overrides
            .get(&combo.combo_name)
            .cloned()
            .unwrap_or_else(|| combo.default.to_string());
        defines.insert(combo.combo_name.clone(), value);
    }

    let options = naga::front::glsl::Options { stage, defines };
    let module = naga::front::glsl::Frontend::default()
        .parse(&options, translated)
        .map_err(|errors| ShaderCompileError::Parse(errors.errors))?;

    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|e| ShaderCompileError::Validate(e.emit_to_string(translated)))?;

    Ok(CompiledShader { module, info })
}

#[derive(Debug)]
pub struct LinkedPair {
    pub vertex: CompiledShader,
    pub fragment: CompiledShader,

    pub vertex_properties: Vec<PropertyDecl>,
    pub fragment_properties: Vec<PropertyDecl>,
}

pub fn compile_pair(
    vertex_source: &str,
    fragment_source: &str,
    combo_overrides: &HashMap<String, String>,
) -> Result<LinkedPair, ShaderCompileError> {
    let vertex_resolved = resolve_includes(vertex_source)?;
    let fragment_resolved = resolve_includes(fragment_source)?;

    let mut combos = parse_combos(&vertex_resolved);
    for combo in parse_combos(&fragment_resolved) {
        if !combos.iter().any(|c| c.combo_name == combo.combo_name) {
            combos.push(combo);
        }
    }
    let vertex_properties = parse_material_properties(&vertex_resolved);
    let fragment_properties = parse_material_properties(&fragment_resolved);

    let mut vertex_body = rewrite_dialect_body(&vertex_resolved, ShaderStage::Vertex);
    let mut fragment_body = rewrite_dialect_body(&fragment_resolved, ShaderStage::Fragment);

    let mut next_binding = 0u32;
    vertex_body = assign_uniform_bindings_from(&vertex_body, &mut next_binding);
    fragment_body = assign_uniform_bindings_from(&fragment_body, &mut next_binding);

    let mut attr_locations = HashMap::new();
    let mut attr_next = 0u32;
    vertex_body = number_named_io(
        &vertex_body,
        "in ",
        &mut attr_locations,
        &mut attr_next,
        false,
    )?;

    let mut varying_locations = HashMap::new();
    let mut varying_next = 0u32;
    vertex_body = number_named_io(
        &vertex_body,
        "out ",
        &mut varying_locations,
        &mut varying_next,
        false,
    )?;
    fragment_body = number_named_io(
        &fragment_body,
        "in ",
        &mut varying_locations,
        &mut varying_next,
        true,
    )?;

    let mut fragment_header = String::new();
    if fragment_body.contains("gl_FragColor") {
        fragment_body = fragment_body.replace("gl_FragColor", "fragColor");
        fragment_header.push_str("out vec4 fragColor;\n");
    }
    let mut out_locations = HashMap::new();
    let mut out_next = 0u32;
    fragment_header = number_named_io(
        &fragment_header,
        "out ",
        &mut out_locations,
        &mut out_next,
        false,
    )?;

    let vertex_final = format!(
        "#version 450 core\n{}{vertex_body}",
        prelude_for(ShaderStage::Vertex)
    );
    let fragment_final = format!(
        "#version 450 core\n{}{fragment_header}{fragment_body}",
        prelude_for(ShaderStage::Fragment)
    );

    let vertex = parse_and_validate(&vertex_final, ShaderStage::Vertex, &combos, combo_overrides)?;
    let fragment = parse_and_validate(
        &fragment_final,
        ShaderStage::Fragment,
        &combos,
        combo_overrides,
    )?;
    Ok(LinkedPair {
        vertex,
        fragment,
        vertex_properties,
        fragment_properties,
    })
}

pub fn to_msl(compiled: &CompiledShader) -> Result<String, ShaderCompileError> {
    let options = naga::back::msl::Options::default();
    let pipeline_options = naga::back::msl::PipelineOptions::default();
    let (msl, _info) = naga::back::msl::write_string(
        &compiled.module,
        &compiled.info,
        &options,
        &pipeline_options,
    )
    .map_err(|e| ShaderCompileError::Msl(e.to_string()))?;
    Ok(msl)
}

pub fn to_wgsl(compiled: &CompiledShader) -> Result<String, ShaderCompileError> {
    naga::back::wgsl::write_string(
        &compiled.module,
        &compiled.info,
        naga::back::wgsl::WriterFlags::empty(),
    )
    .map_err(|e| ShaderCompileError::Wgsl(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_pair_gives_a_matching_varying_the_same_location_on_both_sides() {
        let vert = "attribute vec3 a_Position;\nvarying vec4 v_TexCoord;\nvoid main() { v_TexCoord = vec4(a_Position, 1.0); gl_Position = vec4(a_Position, 1.0); }\n";
        let frag = "varying vec4 v_TexCoord;\nvoid main() { gl_FragColor = v_TexCoord; }\n";
        let pair =
            compile_pair(vert, frag, &HashMap::new()).expect("a matching varying pair should link");

        let _ = (pair.vertex, pair.fragment);
    }

    #[test]
    fn compile_pair_rejects_a_fragment_varying_the_vertex_never_writes() {
        let vert =
            "attribute vec3 a_Position;\nvoid main() { gl_Position = vec4(a_Position, 1.0); }\n";
        let frag = "varying vec4 v_Missing;\nvoid main() { gl_FragColor = v_Missing; }\n";
        let err = compile_pair(vert, frag, &HashMap::new()).unwrap_err();
        match err {
            ShaderCompileError::UnmatchedVarying(name) => assert_eq!(name, "v_Missing"),
            other => panic!("expected UnmatchedVarying, got: {other}"),
        }
    }

    #[test]
    fn compile_pair_applies_a_combo_declared_only_in_the_other_stage_to_both() {
        let vert = "attribute vec3 a_Position;\nvarying vec4 v_TexCoord;\n#if NOISE == 1\nvarying vec4 v_Noise;\n#endif\nvoid main() { gl_Position = vec4(a_Position, 1.0); v_TexCoord = vec4(a_Position, 1.0); \n#if NOISE == 1\nv_Noise = vec4(1.0);\n#endif\n}\n";
        let frag = "// [COMBO] {\"material\":\"m\",\"combo\":\"NOISE\",\"default\":1}\nvarying vec4 v_TexCoord;\n#if NOISE == 1\nvarying vec4 v_Noise;\n#endif\nvoid main() { gl_FragColor = v_TexCoord;\n#if NOISE == 1\ngl_FragColor += v_Noise;\n#endif\n}\n";
        let pair = compile_pair(vert, frag, &HashMap::new())
            .expect("the vertex-side #if NOISE block should see the same NOISE=1 default the fragment's annotation declares");

        let vertex_entry = &pair.vertex.module.entry_points[0];
        let result_ty =
            &pair.vertex.module.types[vertex_entry.function.result.as_ref().unwrap().ty];
        let naga::TypeInner::Struct { members, .. } = &result_ty.inner else {
            panic!("expected a struct result")
        };

        assert_eq!(
            members.len(),
            3,
            "expected v_TexCoord + v_Noise + gl_Position in the vertex output, got: {members:?}"
        );

        let fragment_entry = &pair.fragment.module.entry_points[0];
        assert_eq!(
            fragment_entry.function.arguments.len(),
            2,
            "expected both v_TexCoord and v_Noise as fragment inputs"
        );
    }

    #[test]
    fn parses_a_combo_annotation_but_not_a_plain_property_one() {
        let source = r#"
uniform sampler2D g_Texture0; // {"material":"ui_editor_properties_blend_mode","combo":"BLENDMODE","type":"imageblending","default":0}
uniform float u_DesaturationAmount; // {"material":"Amount","default":1,"range":[0,1]}
"#;
        let combos = parse_combos(source);
        assert_eq!(
            combos.len(),
            1,
            "the non-combo property annotation should not be picked up"
        );
        assert_eq!(combos[0].combo_name, "BLENDMODE");
        assert_eq!(combos[0].uniform_name.as_deref(), Some("g_Texture0"));
        assert_eq!(combos[0].default, 0);
    }

    #[test]
    fn parses_real_material_property_annotations() {
        let source = r#"
uniform float g_BlendAlpha; // {"material":"alpha","label":"ui_editor_properties_alpha","default":1,"range":[0.01,1]}
uniform vec3 g_OutlineColor1; // {"material":"outlinecolor","label":"ui_editor_properties_outline_color","default":"0 0 0","type":"color"}
uniform sampler2D g_Texture0; // {"hidden":true}
"#;
        let props = parse_material_properties(source);
        assert_eq!(
            props.len(),
            2,
            "the hidden g_Texture0 annotation has no \"material\" key and shouldn't be picked up"
        );
        assert_eq!(props[0].uniform_name, "g_BlendAlpha");
        assert_eq!(props[0].material_key, "alpha");
        assert_eq!(props[0].glsl_type, "float");
        assert_eq!(props[1].uniform_name, "g_OutlineColor1");
        assert_eq!(props[1].material_key, "outlinecolor");
        assert_eq!(props[1].glsl_type, "vec3");
    }

    #[test]
    fn parses_a_real_standalone_combo_annotation_with_the_combo_tag() {
        let source = r#"
// [COMBO] {"material":"ui_editor_properties_blend_mode","combo":"BLENDMODE","type":"imageblending","default":30}

#include "common_blending.h"
"#;
        let combos = parse_combos(source);
        assert_eq!(combos.len(), 1);
        assert_eq!(combos[0].combo_name, "BLENDMODE");
        assert_eq!(combos[0].uniform_name, None);
        assert_eq!(combos[0].default, 30);
    }

    #[test]
    fn transpiles_attribute_varying_and_gl_fragcolor() {
        let vert = "attribute vec3 a_Position;\nvarying vec4 v_TexCoord;\nvoid main() { v_TexCoord = vec4(a_Position, 1.0); }\n";
        let out = transpile_dialect(vert, ShaderStage::Vertex);
        assert!(out.starts_with("#version 450 core\n"));
        assert!(out.contains("in vec3 a_Position;"), "got:\n{out}");
        assert!(out.contains("out vec4 v_TexCoord;"), "got:\n{out}");
        assert!(
            out.contains("layout(location="),
            "expected explicit locations, got:\n{out}"
        );

        let frag = "varying vec4 v_TexCoord;\nvoid main() { gl_FragColor = v_TexCoord; }\n";
        let out = transpile_dialect(frag, ShaderStage::Fragment);
        assert!(out.contains("in vec4 v_TexCoord;"), "got:\n{out}");
        assert!(out.contains("out vec4 fragColor;"), "got:\n{out}");
        assert!(out.contains("fragColor = v_TexCoord;"), "got:\n{out}");
        assert!(!out.contains("gl_FragColor"));
    }

    #[test]
    fn drops_precision_and_extension_lines() {
        let frag = "precision highp float;\n#extension GL_OES_standard_derivatives : enable\nvoid main() { gl_FragColor = vec4(1.0); }\n";
        let out = transpile_dialect(frag, ShaderStage::Fragment);

        assert!(
            !out.lines()
                .any(|l| l.trim_start().starts_with("precision ")),
            "got:\n{out}"
        );
        assert!(
            !out.lines()
                .any(|l| l.trim_start().starts_with("#extension")),
            "got:\n{out}"
        );
    }

    #[test]
    fn rewrites_texture_sample_call_suffixes() {
        let frag = "void main() { gl_FragColor = texture2D(g_Texture0, uv) + texture2DLod(g_Texture0, uv, 0.0); }\n";
        let out = transpile_dialect(frag, ShaderStage::Fragment);
        assert!(out.contains("texture(g_Texture0"), "got:\n{out}");
        assert!(out.contains("textureLod(g_Texture0"), "got:\n{out}");

        assert!(!out.contains("texture2D(g_Texture0"), "got:\n{out}");
    }

    #[test]
    fn renames_the_reserved_sample_identifier_but_not_lookalike_names() {
        let frag = "void main() { \
            vec4 sample = texture2D(g_Texture0, uv); \
            float sampleCount = 30.0; \
            albedo += sample * sampleCount; \
            gl_FragColor = sample; \
        }\n";
        let out = transpile_dialect(frag, ShaderStage::Fragment);
        assert!(
            !out.contains("vec4 sample "),
            "the reserved word itself should be renamed, got:\n{out}"
        );
        assert!(
            out.contains("we_sample_local"),
            "expected the renamed identifier to appear, got:\n{out}"
        );

        assert!(
            out.contains("sampleCount"),
            "a lookalike identifier shouldn't be touched, got:\n{out}"
        );
    }
}
