use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum SceneError {
    #[error("io error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid scene.json: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("unsupported format_version {0}")]
    UnsupportedVersion(u32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    pub format_version: u32,
    pub title: String,
    #[serde(flatten)]
    pub kind: SceneKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SceneKind {
    Video {
        video: VideoLayer,
    },

    Layered {
        canvas_width: f32,
        canvas_height: f32,
        #[serde(default)]
        parallax: Parallax,
        layers: Vec<Layer>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Parallax {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub amount: f32,
    #[serde(default)]
    pub mouse_influence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoLayer {
    pub asset: PathBuf,
    #[serde(default = "default_true")]
    pub r#loop: bool,
    #[serde(default = "default_true")]
    pub muted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layer {
    pub id: String,
    #[serde(flatten)]
    pub content: LayerContent,
    #[serde(default)]
    pub transform: Transform,
    #[serde(default)]
    pub blend: BlendMode,
    #[serde(default)]
    pub effects: Vec<Effect>,

    #[serde(default = "one")]
    pub parallax_depth: f32,

    #[serde(default)]
    pub parallax_depth_y: f32,

    #[serde(default)]
    pub we_effect_passes: Vec<EffectPass>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectPass {
    pub vert_source: String,
    pub frag_source: String,

    #[serde(default)]
    pub combo_overrides: std::collections::HashMap<String, String>,

    #[serde(default)]
    pub constants: std::collections::HashMap<String, ConstantValue>,

    #[serde(default)]
    pub textures: Vec<Option<PathBuf>>,

    #[serde(default)]
    pub effect_name: String,

    #[serde(default)]
    pub target: Option<String>,

    #[serde(default)]
    pub bind: Vec<PassBind>,

    #[serde(default)]
    pub fbos: Vec<NamedFbo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PassBind {
    pub name: String,
    pub index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedFbo {
    pub name: String,

    pub scale: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConstantValue {
    Scalar(f32),
    Vector(Vec<f32>),
}

fn one() -> f32 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayerContent {
    Video { asset: PathBuf },
    Image { asset: PathBuf },
    Shader { asset: PathBuf },

    Audio { asset: PathBuf, volume: f32 },

    Particles(ParticleSystem),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticleSystem {
    pub maxcount: u32,
    #[serde(default)]
    pub starttime: f32,

    pub material: String,

    #[serde(default)]
    pub texture: Option<PathBuf>,

    #[serde(default)]
    pub origin: (f32, f32),
    #[serde(default = "one")]
    pub scale: f32,
    #[serde(default)]
    pub rotation: f32,

    #[serde(default, rename = "controlpoint")]
    pub controlpoints: Vec<ParticleControlPoint>,
    #[serde(default, rename = "emitter")]
    pub emitters: Vec<ParticleModule>,
    #[serde(default, rename = "initializer")]
    pub initializers: Vec<ParticleModule>,
    #[serde(default, rename = "operator")]
    pub operators: Vec<ParticleModule>,
    #[serde(default, rename = "renderer")]
    pub renderers: Vec<ParticleModule>,

    #[serde(default)]
    pub children: Vec<ParticleChild>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticleControlPoint {
    pub id: u32,
    #[serde(default)]
    pub flags: i64,

    #[serde(default)]
    pub offset: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticleModule {
    pub id: u32,
    pub name: String,

    #[serde(flatten)]
    pub params: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticleChild {
    pub id: u32,

    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub maxcount: Option<u32>,

    pub name: String,

    pub system: Option<Box<ParticleSystem>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transform {
    pub x: f32,
    pub y: f32,
    #[serde(default)]
    pub width: f32,
    #[serde(default)]
    pub height: f32,
    #[serde(default)]
    pub rotation: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            rotation: 0.0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    #[default]
    Normal,
    Add,
    Multiply,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Effect {
    BrightnessContrast { brightness: f32, contrast: f32 },
    Saturation { amount: f32 },
    Blur { radius: f32 },
    Tint { color: [f32; 3], amount: f32 },
    Vignette { amount: f32 },
}

fn default_true() -> bool {
    true
}

impl Scene {
    pub fn load_dir(dir: impl AsRef<Path>) -> Result<Self, SceneError> {
        let dir = dir.as_ref();
        let path = dir.join("scene.json");
        let raw = std::fs::read_to_string(&path).map_err(|source| SceneError::Io {
            path: path.clone(),
            source,
        })?;
        let scene: Scene = serde_json::from_str(&raw)?;
        if scene.format_version != 1 {
            return Err(SceneError::UnsupportedVersion(scene.format_version));
        }
        Ok(scene)
    }

    pub fn primary_video_path(&self, scene_dir: impl AsRef<Path>) -> Option<PathBuf> {
        match &self.kind {
            SceneKind::Video { video } => Some(resolve_asset(scene_dir.as_ref(), &video.asset)),
            SceneKind::Layered { .. } => None,
        }
    }

    pub fn canvas_size(&self) -> Option<(f32, f32)> {
        match &self.kind {
            SceneKind::Video { .. } => None,
            SceneKind::Layered {
                canvas_width,
                canvas_height,
                ..
            } => Some((*canvas_width, *canvas_height)),
        }
    }

    pub fn asset_paths(&self, scene_dir: impl AsRef<Path>) -> Vec<PathBuf> {
        let scene_dir = scene_dir.as_ref();
        match &self.kind {
            SceneKind::Video { video } => vec![resolve_asset(scene_dir, &video.asset)],
            SceneKind::Layered { layers, .. } => layers
                .iter()
                .filter_map(|l| match &l.content {
                    LayerContent::Video { asset }
                    | LayerContent::Image { asset }
                    | LayerContent::Shader { asset }
                    | LayerContent::Audio { asset, .. } => Some(resolve_asset(scene_dir, asset)),

                    LayerContent::Particles(_) => None,
                })
                .collect(),
        }
    }

    pub fn is_muted(&self) -> bool {
        match &self.kind {
            SceneKind::Video { video } => video.muted,
            SceneKind::Layered { .. } => true,
        }
    }
}

fn resolve_asset(scene_dir: &Path, asset: &Path) -> PathBuf {
    if asset.is_absolute() {
        asset.to_path_buf()
    } else {
        scene_dir.join(asset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_video_scene() {
        let json = r#"{
            "format_version": 1,
            "title": "Rainy City Loop",
            "kind": "video",
            "video": { "asset": "assets/background.mp4" }
        }"#;
        let scene: Scene = serde_json::from_str(json).unwrap();
        assert_eq!(scene.title, "Rainy City Loop");
        match scene.kind {
            SceneKind::Video { video } => {
                assert!(video.r#loop);
                assert!(video.muted);
                assert_eq!(video.asset, PathBuf::from("assets/background.mp4"));
            }
            _ => panic!("expected video scene"),
        }
    }
}
