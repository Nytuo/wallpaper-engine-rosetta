use crate::model::{
    ConstantValue, EffectPass, Layer, LayerContent, NamedFbo, Parallax, ParticleSystem, PassBind,
    Scene, SceneKind, Transform, VideoLayer,
};
use crate::we_pkg::{self, PkgEntry};
use crate::we_tex;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum WeCompatError {
    #[error("io error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid project.json: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("project.json has no recognizable content (no `file`, no scene layers)")]
    NoContent,
    #[error("failed to read scene.pkg: {0}")]
    Pkg(#[from] we_pkg::PkgError),
    #[error(
        "\"{title}\" is a Wallpaper Engine \"Scene\" item, but none of its layers could be translated \
         (no plain static-image layer found, or its texture uses a format this project doesn't decode yet — \
         see we_tex's docs for which ones it does). Full WE scene rendering (shaders, particles, parallax) \
         isn't supported — only a static-image approximation is attempted."
    )]
    UnsupportedPackedScene { title: String },
}

#[derive(Debug)]
pub struct WeImportResult {
    pub scene: Scene,
    pub skipped: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct WeProject {
    title: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,

    file: Option<String>,
}

pub fn import_dir(dir: impl AsRef<Path>) -> Result<WeImportResult, WeCompatError> {
    let dir = dir.as_ref();
    let project_path = dir.join("project.json");
    let raw = std::fs::read_to_string(&project_path).map_err(|source| WeCompatError::Io {
        path: project_path.clone(),
        source,
    })?;
    let project: WeProject = serde_json::from_str(&raw)?;
    let title = project
        .title
        .clone()
        .unwrap_or_else(|| "Untitled".to_string());

    let kind_lower = project.kind.as_deref().map(str::to_lowercase);

    if kind_lower.as_deref() == Some("video") {
        if let Some(file) = &project.file {
            return Ok(WeImportResult {
                scene: Scene {
                    format_version: 1,
                    title,
                    kind: SceneKind::Video {
                        video: VideoLayer {
                            asset: dir.join(file),
                            r#loop: true,
                            muted: true,
                        },
                    },
                },
                skipped: Vec::new(),
            });
        }
    }

    let pkg_path = dir.join("scene.pkg");
    if kind_lower.as_deref() == Some("scene") && pkg_path.exists() {
        return import_packed_scene(dir, &pkg_path, title);
    }

    let scene_json_path = dir.join("scene.json");
    if scene_json_path.exists() {
        let raw =
            std::fs::read_to_string(&scene_json_path).map_err(|source| WeCompatError::Io {
                path: scene_json_path.clone(),
                source,
            })?;
        let root: Value = serde_json::from_str(&raw)?;
        return translate_scene_graph(&root, title, dir, &AssetSource::Disk(dir.to_path_buf()));
    }

    Err(WeCompatError::NoContent)
}

enum AssetSource {
    Packed(HashMap<String, Vec<u8>>),
    Disk(PathBuf),
}

impl AssetSource {
    fn read(&self, relative_path: &str) -> Option<Vec<u8>> {
        match self {
            AssetSource::Packed(entries) => entries.get(relative_path).cloned(),
            AssetSource::Disk(dir) => std::fs::read(dir.join(relative_path)).ok(),
        }
    }
}

fn import_packed_scene(
    dir: &Path,
    pkg_path: &Path,
    title: String,
) -> Result<WeImportResult, WeCompatError> {
    let entries: HashMap<String, Vec<u8>> = we_pkg::extract(pkg_path)?
        .into_iter()
        .map(|PkgEntry { path, data }| (path, data))
        .collect();

    let scene_json_bytes = entries.get("scene.json").ok_or(WeCompatError::NoContent)?;
    let root: Value = serde_json::from_slice(scene_json_bytes)?;

    translate_scene_graph(&root, title, dir, &AssetSource::Packed(entries))
}

pub fn parallax_depth_of(object: &Value) -> (f32, f32) {
    if let Some((x, y)) = parse_vec_prefix(object.get("parallaxDepth").and_then(Value::as_str)) {
        return (x as f32, y as f32);
    }
    let locked = object
        .get("locktransforms")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if locked {
        (0.0, 0.0)
    } else {
        (1.0, 1.0)
    }
}

fn translate_scene_graph(
    root: &Value,
    title: String,
    item_dir: &Path,
    assets: &AssetSource,
) -> Result<WeImportResult, WeCompatError> {
    let (canvas_width, canvas_height) = root
        .pointer("/general/orthogonalprojection")
        .and_then(|p| Some((p.get("width")?.as_f64()?, p.get("height")?.as_f64()?)))
        .unwrap_or((1920.0, 1080.0));

    let parallax = Parallax {
        enabled: root
            .pointer("/general/cameraparallax")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        amount: root
            .pointer("/general/cameraparallaxamount")
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as f32,
        mouse_influence: root
            .pointer("/general/cameraparallaxmouseinfluence")
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as f32,
    };

    let extracted_dir = item_dir.join("extracted");
    let mut layers = Vec::new();
    let mut skipped = Vec::new();

    let objects = root
        .get("objects")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    for (index, obj) in objects.iter().enumerate() {
        let name = obj
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("object");

        if obj.get("visible").and_then(Value::as_bool) == Some(false) {
            skipped.push(format!("layer '{name}': hidden, skipped"));
            continue;
        }
        if let Some(particle_ref) = obj.get("particle").and_then(Value::as_str) {
            match resolve_particle_system(assets, particle_ref, &extracted_dir, index, 0) {
                Ok(mut system) => {
                    let (ox, oy) = parse_vec_prefix(obj.get("origin").and_then(Value::as_str))
                        .unwrap_or((0.0, 0.0));
                    let (sx, _sy) = parse_vec_prefix(obj.get("scale").and_then(Value::as_str))
                        .unwrap_or((1.0, 1.0));
                    let angles = obj.get("angles").and_then(Value::as_str).unwrap_or("0 0 0");
                    let rotation_z = angles
                        .split_whitespace()
                        .nth(2)
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    system.origin = (ox as f32, oy as f32);
                    system.scale = sx as f32;
                    system.rotation = rotation_z as f32;

                    layers.push(Layer {
                        id: format!("we-layer-{index}"),
                        content: LayerContent::Particles(system),
                        transform: Transform::default(),
                        blend: Default::default(),
                        effects: Vec::new(),
                        parallax_depth: 0.0,
                        parallax_depth_y: 0.0,
                        we_effect_passes: Vec::new(),
                    });
                }
                Err(reason) => skipped.push(format!(
                    "layer '{name}': particle system not translated: {reason}"
                )),
            }
            continue;
        }
        if let Some(sound_ref) = obj
            .get("sound")
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(Value::as_str)
        {
            let volume = obj.get("volume").and_then(Value::as_f64).unwrap_or(1.0) as f32;
            match resolve_layer_audio(assets, sound_ref, &extracted_dir, index) {
                Ok(asset_path) => {
                    layers.push(Layer {
                        id: format!("we-layer-{index}"),
                        content: LayerContent::Audio {
                            asset: asset_path,
                            volume,
                        },
                        transform: Transform::default(),
                        blend: Default::default(),
                        effects: Vec::new(),
                        parallax_depth: 0.0,
                        parallax_depth_y: 0.0,
                        we_effect_passes: Vec::new(),
                    });
                }
                Err(reason) => skipped.push(format!(
                    "layer '{name}': audio track not translated: {reason}"
                )),
            }
            continue;
        }
        if obj.get("sound").is_some() {
            skipped.push(format!(
                "layer '{name}': audio track has no usable \"sound\" entry, skipped"
            ));
            continue;
        }
        let Some(image_ref) = obj.get("image").and_then(Value::as_str) else {
            skipped.push(format!(
                "layer '{name}': not a plain image layer (particles/effects-only/etc), skipped"
            ));
            continue;
        };

        match resolve_layer_image(assets, image_ref, &extracted_dir, index) {
            Ok(asset_path) => {
                let (x, y) = parse_vec_prefix(obj.get("origin").and_then(Value::as_str))
                    .unwrap_or((0.0, 0.0));
                let (base_width, base_height) =
                    parse_vec_prefix(obj.get("size").and_then(Value::as_str)).unwrap_or((0.0, 0.0));

                let (scale_x, scale_y) = parse_vec_prefix(obj.get("scale").and_then(Value::as_str))
                    .unwrap_or((1.0, 1.0));
                let (width, height) = (base_width * scale_x, base_height * scale_y);

                let mut we_effect_passes = Vec::new();
                if let Some(effects) = obj.get("effects").and_then(Value::as_array) {
                    for (effect_index, effect) in effects.iter().enumerate() {
                        if effect.get("visible").and_then(Value::as_bool) == Some(false) {
                            continue;
                        }
                        let effect_name = effect
                            .get("name")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty());
                        let Some(effect_file) = effect.get("file").and_then(Value::as_str) else {
                            skipped.push(format!(
                                "layer '{name}': effect #{effect_index} has no \"file\" reference"
                            ));
                            continue;
                        };
                        let Some(passes) = effect.get("passes").and_then(Value::as_array) else {
                            skipped.push(format!(
                                "layer '{name}': effect \"{effect_file}\" has no \"passes\""
                            ));
                            continue;
                        };
                        for (pass_index, pass) in passes.iter().enumerate() {
                            let label = effect_name
                                .map(str::to_string)
                                .unwrap_or_else(|| effect_file.to_string());
                            match parse_effect_pass(assets, effect_file, pass, &extracted_dir, index, effect_index, pass_index) {
                                Ok(effect_pass) => we_effect_passes.push(effect_pass),
                                Err(reason) => skipped.push(format!(
                                    "layer '{name}': effect \"{label}\" pass #{pass_index} not translated: {reason}"
                                )),
                            }
                        }
                    }
                }

                let (depth_x, depth_y) = parallax_depth_of(obj);

                layers.push(Layer {
                    id: format!("we-layer-{index}"),
                    content: LayerContent::Image { asset: asset_path },
                    transform: Transform {
                        x: x as f32,
                        y: y as f32,
                        width: width as f32,
                        height: height as f32,
                        rotation: 0.0,
                    },
                    blend: Default::default(),
                    effects: Vec::new(),
                    parallax_depth: depth_x,
                    parallax_depth_y: depth_y,
                    we_effect_passes,
                });
            }
            Err(reason) => {
                skipped.push(format!("layer '{name}': {reason}"));
            }
        }
    }

    if layers.is_empty() {
        return Err(WeCompatError::UnsupportedPackedScene { title });
    }

    Ok(WeImportResult {
        scene: Scene {
            format_version: 1,
            title,
            kind: SceneKind::Layered {
                canvas_width: canvas_width as f32,
                canvas_height: canvas_height as f32,
                parallax,
                layers,
            },
        },
        skipped,
    })
}

fn resolve_layer_image(
    assets: &AssetSource,
    image_ref: &str,
    extracted_dir: &Path,
    index: usize,
) -> Result<PathBuf, String> {
    let model_bytes = assets
        .read(image_ref)
        .ok_or_else(|| format!("referenced model \"{image_ref}\" not found"))?;
    let model: Value =
        serde_json::from_slice(&model_bytes).map_err(|e| format!("invalid model json: {e}"))?;

    let material_ref = model
        .get("material")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            format!("model \"{image_ref}\" has no \"material\" field (not a plain image model)")
        })?;

    let material_bytes = assets
        .read(material_ref)
        .ok_or_else(|| format!("referenced material \"{material_ref}\" not found"))?;
    let material: Value = serde_json::from_slice(&material_bytes)
        .map_err(|e| format!("invalid material json: {e}"))?;

    let texture_name = material
        .pointer("/passes/0/textures")
        .and_then(Value::as_array)
        .and_then(|textures| textures.iter().find_map(Value::as_str))
        .ok_or_else(|| format!("material \"{material_ref}\" has no texture reference"))?;

    decode_and_write_texture(
        assets,
        texture_name,
        extracted_dir,
        &format!("layer-{index}.png"),
    )
}

fn resolve_layer_audio(
    assets: &AssetSource,
    sound_ref: &str,
    extracted_dir: &Path,
    index: usize,
) -> Result<PathBuf, String> {
    let bytes = assets
        .read(sound_ref)
        .ok_or_else(|| format!("referenced audio \"{sound_ref}\" not found"))?;
    let ext = Path::new(sound_ref)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp3");

    std::fs::create_dir_all(extracted_dir)
        .map_err(|e| format!("could not create extracted-assets dir: {e}"))?;
    let out_path = extracted_dir.join(format!("layer-{index}-audio.{ext}"));
    std::fs::write(&out_path, &bytes)
        .map_err(|e| format!("could not write extracted audio: {e}"))?;
    Ok(out_path)
}

const MAX_PARTICLE_CHILD_DEPTH: u32 = 8;

fn resolve_particle_system(
    assets: &AssetSource,
    particle_ref: &str,
    extracted_dir: &Path,
    layer_index: usize,
    depth: u32,
) -> Result<ParticleSystem, String> {
    if depth > MAX_PARTICLE_CHILD_DEPTH {
        return Err(format!("particle system nesting exceeded {MAX_PARTICLE_CHILD_DEPTH} levels, refusing to recurse further"));
    }
    let bytes = assets
        .read(particle_ref)
        .ok_or_else(|| format!("particle definition \"{particle_ref}\" not found"))?;
    let mut value: Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("invalid particle json \"{particle_ref}\": {e}"))?;

    if matches!(value.get("children"), Some(Value::Null)) {
        if let Value::Object(map) = &mut value {
            map.remove("children");
        }
    }

    if let Some(children) = value.get_mut("children").and_then(Value::as_array_mut) {
        for child in children.iter_mut() {
            let Some(child_ref) = child
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                continue;
            };
            let resolved =
                resolve_particle_system(assets, &child_ref, extracted_dir, layer_index, depth + 1);
            if let (Value::Object(map), Ok(system)) = (&mut *child, resolved) {
                map.insert(
                    "system".to_string(),
                    serde_json::to_value(system).unwrap_or(Value::Null),
                );
            }
        }
    }

    if let Some(material_ref) = value
        .get("material")
        .and_then(Value::as_str)
        .map(str::to_string)
    {
        let unique: String = particle_ref
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        match resolve_particle_material_texture(
            assets,
            &material_ref,
            extracted_dir,
            &format!("particle-{layer_index}-{unique}.png"),
        ) {
            Ok(path) => {
                if let Value::Object(map) = &mut value {
                    map.insert(
                        "texture".to_string(),
                        Value::String(path.display().to_string()),
                    );
                }
            }
            Err(_) => {}
        }
    }

    serde_json::from_value(value).map_err(|e| {
        format!("particle json \"{particle_ref}\" doesn't match the expected real shape: {e}")
    })
}

fn resolve_particle_material_texture(
    assets: &AssetSource,
    material_ref: &str,
    extracted_dir: &Path,
    out_filename: &str,
) -> Result<PathBuf, String> {
    let material_bytes = assets
        .read(material_ref)
        .ok_or_else(|| format!("particle material \"{material_ref}\" not found"))?;
    let material: Value = serde_json::from_slice(&material_bytes)
        .map_err(|e| format!("invalid particle material json: {e}"))?;
    let texture_name = material
        .pointer("/passes/0/textures")
        .and_then(Value::as_array)
        .and_then(|textures| textures.iter().find_map(Value::as_str))
        .ok_or_else(|| format!("particle material \"{material_ref}\" has no texture reference"))?;
    decode_and_write_texture(assets, texture_name, extracted_dir, out_filename)
}

fn decode_and_write_texture(
    assets: &AssetSource,
    texture_name: &str,
    extracted_dir: &Path,
    out_filename: &str,
) -> Result<PathBuf, String> {
    let texture_path = format!("materials/{texture_name}.tex");

    let png = if let Some(texture_bytes) = assets.read(&texture_path) {
        we_tex::extract_primary_png(&texture_bytes)
            .map_err(|e| format!("texture \"{texture_path}\" not decodable: {e}"))?
    } else {
        resolve_we_stock_texture(texture_name)
            .ok_or_else(|| format!("texture file \"{texture_path}\" not found"))?
    };

    std::fs::create_dir_all(extracted_dir)
        .map_err(|e| format!("could not create extracted-assets dir: {e}"))?;
    let out_path = extracted_dir.join(out_filename);
    std::fs::write(&out_path, &png).map_err(|e| format!("could not write extracted image: {e}"))?;
    Ok(out_path)
}

fn we_assets_fallback_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(configured) = std::env::var_os("WER_ASSETS_DIR") {
        let dir = PathBuf::from(configured);
        if dir.is_dir() {
            dirs.push(dir);
        }
    }
    dirs
}

fn resolve_we_stock_texture(texture_name: &str) -> Option<Vec<u8>> {
    for dir in we_assets_fallback_dirs() {
        let tex_path = dir.join(format!("materials/{texture_name}.tex"));
        if let Ok(bytes) = std::fs::read(&tex_path) {
            if let Ok(png) = we_tex::extract_primary_png(&bytes) {
                return Some(png);
            }
        }
        let png_path = dir.join(format!("materials/{texture_name}.png"));
        if let Ok(png) = std::fs::read(&png_path) {
            return Some(png);
        }
    }
    None
}

fn parse_effect_pass(
    assets: &AssetSource,
    effect_file: &str,
    pass: &Value,
    extracted_dir: &Path,
    layer_index: usize,
    effect_index: usize,
    pass_index: usize,
) -> Result<EffectPass, String> {
    let effect_json_bytes = assets
        .read(effect_file)
        .ok_or_else(|| format!("effect definition \"{effect_file}\" not found"))?;
    let effect_json: Value = serde_json::from_slice(&effect_json_bytes)
        .map_err(|e| format!("invalid effect json \"{effect_file}\": {e}"))?;
    let effect_name = effect_json
        .get("replacementkey")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let material_ref = effect_json
        .pointer(&format!("/passes/{pass_index}/material"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            format!("effect \"{effect_file}\" has no \"passes[{pass_index}].material\"")
        })?;
    let material_bytes = assets
        .read(material_ref)
        .ok_or_else(|| format!("material \"{material_ref}\" not found"))?;
    let material_json: Value = serde_json::from_slice(&material_bytes)
        .map_err(|e| format!("invalid material json \"{material_ref}\": {e}"))?;
    let shader_base = material_json
        .pointer("/passes/0/shader")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("material \"{material_ref}\" has no \"passes[0].shader\""))?;
    let vert_path = format!("shaders/{shader_base}.vert");
    let frag_path = format!("shaders/{shader_base}.frag");

    let read_source = |path: &str| -> Result<String, String> {
        let bytes = assets
            .read(path)
            .ok_or_else(|| format!("shader file \"{path}\" not found"))?;
        String::from_utf8(bytes)
            .map_err(|e| format!("shader file \"{path}\" isn't valid UTF-8: {e}"))
    };
    let vert_source = read_source(&vert_path)?;
    let frag_source = read_source(&frag_path)?;

    let combo_overrides = pass
        .get("combos")
        .and_then(Value::as_object)
        .map(|combos| {
            combos
                .iter()
                .filter_map(|(name, value)| value.as_i64().map(|n| (name.clone(), n.to_string())))
                .collect()
        })
        .unwrap_or_default();

    let constants = pass
        .get("constantshadervalues")
        .and_then(Value::as_object)
        .map(|values| {
            values
                .iter()
                .filter_map(|(key, value)| parse_constant_value(value).map(|cv| (key.clone(), cv)))
                .collect()
        })
        .unwrap_or_default();

    let mut textures = Vec::new();
    if let Some(texture_refs) = pass.get("textures").and_then(Value::as_array) {
        for (texture_index, entry) in texture_refs.iter().enumerate() {
            match entry.as_str() {
                None => textures.push(None),
                Some(name) => {
                    let out_filename = format!("layer-{layer_index}-effect-{effect_index}-{pass_index}-tex{texture_index}.png");
                    let path = decode_and_write_texture(assets, name, extracted_dir, &out_filename)
                        .map_err(|e| format!("mask texture \"{name}\" not decodable: {e}"))?;
                    textures.push(Some(path));
                }
            }
        }
    }

    let target = effect_json
        .pointer(&format!("/passes/{pass_index}/target"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let bind = effect_json
        .pointer(&format!("/passes/{pass_index}/bind"))
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| {
                    let name = e.get("name")?.as_str()?.to_string();
                    let index = e.get("index")?.as_u64()? as usize;
                    Some(PassBind { name, index })
                })
                .collect()
        })
        .unwrap_or_default();
    let fbos = effect_json
        .get("fbos")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| {
                    let name = e.get("name")?.as_str()?.to_string();
                    let scale = e.get("scale").and_then(Value::as_u64).unwrap_or(1) as u32;
                    Some(NamedFbo { name, scale })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(EffectPass {
        vert_source,
        frag_source,
        combo_overrides,
        constants,
        textures,
        effect_name,
        target,
        bind,
        fbos,
    })
}

fn parse_constant_value(value: &Value) -> Option<ConstantValue> {
    if let Some(n) = value.as_f64() {
        return Some(ConstantValue::Scalar(n as f32));
    }
    let s = value.as_str()?;
    let components: Option<Vec<f32>> = s.split_whitespace().map(|part| part.parse().ok()).collect();
    components.map(ConstantValue::Vector)
}

fn parse_vec_prefix(s: Option<&str>) -> Option<(f64, f64)> {
    let mut parts = s?.split_whitespace();
    let a = parts.next()?.parse().ok()?;
    let b = parts.next()?.parse().ok()?;
    Some((a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_we_vector_strings() {
        assert_eq!(
            parse_vec_prefix(Some("1920.00000 1080.00000 0.00000")),
            Some((1920.0, 1080.0))
        );
        assert_eq!(
            parse_vec_prefix(Some("3840.00000 2160.00000")),
            Some((3840.0, 2160.0))
        );
        assert_eq!(parse_vec_prefix(None), None);
        assert_eq!(parse_vec_prefix(Some("")), None);
    }

    #[test]
    fn resolves_a_stock_texture_from_the_user_supplied_fallback_directory() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let fallback_dir = std::path::PathBuf::from(home)
            .join("Library/Application Support/wer/we_assets/materials/util");
        std::fs::create_dir_all(&fallback_dir).unwrap();
        let png_path = fallback_dir.join("wer_test_marker.png");

        let tiny_png: [u8; 69] = [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D,
            0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        std::fs::write(&png_path, tiny_png).unwrap();

        let resolved = resolve_we_stock_texture("util/wer_test_marker");

        let _ = std::fs::remove_file(&png_path);

        assert_eq!(
            resolved.as_deref(),
            Some(&tiny_png[..]),
            "should read the synthetic PNG straight through"
        );
        assert!(
            resolve_we_stock_texture("util/wer_definitely_does_not_exist").is_none(),
            "a name with no real file in the fallback dir should resolve to None"
        );
    }

    static WER_ASSETS_DIR_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn wer_assets_dir_env_var_is_checked_before_the_legacy_fixed_paths() {
        let _guard = WER_ASSETS_DIR_ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("wer-assets-test-{}", std::process::id()));
        let materials = dir.join("materials/util");
        std::fs::create_dir_all(&materials).unwrap();
        std::fs::write(
            materials.join("marker.png"),
            b"not a real png, just a marker",
        )
        .unwrap();

        unsafe { std::env::set_var("WER_ASSETS_DIR", &dir) };
        let resolved = resolve_we_stock_texture("util/marker");
        unsafe { std::env::remove_var("WER_ASSETS_DIR") };
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(
            resolved.as_deref(),
            Some(&b"not a real png, just a marker"[..])
        );
    }

    #[test]
    fn an_unset_or_nonexistent_wer_assets_dir_is_silently_skipped() {
        let _guard = WER_ASSETS_DIR_ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("WER_ASSETS_DIR", "/definitely/does/not/exist/anywhere") };
        let dirs = we_assets_fallback_dirs();
        unsafe { std::env::remove_var("WER_ASSETS_DIR") };
        assert!(
            dirs.iter()
                .all(|d| d != std::path::Path::new("/definitely/does/not/exist/anywhere")),
            "a configured directory that isn't real must not be searched"
        );
    }
}
