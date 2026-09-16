use crate::model::{Scene, SceneKind, VideoLayer};
use crate::transcode::{self, TranscodeError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum AssignmentError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("(de)serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("could not determine a per-user application data directory on this platform")]
    NoAppDataDir,
    #[error("no wallpaper is currently assigned to display {0}")]
    NotAssigned(String),
    #[error("{0}")]
    Transcode(#[from] TranscodeError),
    #[error("{0}")]
    Scene(#[from] crate::model::SceneError),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EffectSettings {
    #[serde(default)]
    pub brightness: f32,

    #[serde(default = "one")]
    pub contrast: f32,

    #[serde(default = "one")]
    pub saturation: f32,

    #[serde(default)]
    pub blur_radius: f32,

    #[serde(default)]
    pub tint_color: Option<[f32; 3]>,

    #[serde(default)]
    pub tint_amount: f32,

    #[serde(default = "one")]
    pub effect_speed: f32,
}

fn one() -> f32 {
    1.0
}

impl Default for EffectSettings {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            blur_radius: 0.0,
            tint_color: None,
            tint_amount: 0.0,
            effect_speed: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayAssignment {
    pub scene: String,
    #[serde(default = "default_true")]
    pub muted: bool,
    #[serde(default = "one")]
    pub volume: f32,
    #[serde(default)]
    pub effects: EffectSettings,

    #[serde(default)]
    pub playlist: Option<Playlist>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub entries: Vec<String>,

    pub interval_secs: u64,
    #[serde(default)]
    pub shuffle: bool,

    #[serde(default)]
    pub current_index: usize,

    #[serde(default)]
    pub last_advanced_at: u64,
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ShellConfig {
    pub displays: HashMap<String, DisplayAssignment>,

    #[serde(default)]
    pub paused: bool,
}

pub fn config_path() -> Result<PathBuf, AssignmentError> {
    Ok(app_data_dir()?.join("config.json"))
}

fn app_data_dir() -> Result<PathBuf, AssignmentError> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").ok_or(AssignmentError::NoAppDataDir)?;
        Ok(PathBuf::from(home)
            .join("Library/Application Support")
            .join("wer"))
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var_os("APPDATA").ok_or(AssignmentError::NoAppDataDir)?;
        Ok(PathBuf::from(appdata).join("wer"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Err(AssignmentError::NoAppDataDir)
    }
}

pub fn load_config() -> Result<ShellConfig, AssignmentError> {
    let path = config_path()?;
    match std::fs::read_to_string(&path) {
        Ok(raw) => Ok(serde_json::from_str(&raw)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ShellConfig::default()),
        Err(e) => Err(e.into()),
    }
}

pub fn save_config(config: &ShellConfig) -> Result<(), AssignmentError> {
    let path = config_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(config)?)?;
    Ok(())
}

pub fn assign_video(
    display_id: &str,
    video_path: impl AsRef<Path>,
    scenes_dir: impl AsRef<Path>,
) -> Result<PathBuf, AssignmentError> {
    let scene_dir = import_video_to_library(video_path, &scenes_dir)?;

    let mut config = load_config()?;
    config.displays.insert(
        display_id.to_string(),
        DisplayAssignment {
            scene: scene_dir.display().to_string(),
            muted: true,
            volume: 1.0,
            effects: EffectSettings::default(),
            playlist: None,
        },
    );
    save_config(&config)?;

    Ok(scene_dir)
}

pub fn import_video_to_library(
    video_path: impl AsRef<Path>,
    scenes_dir: impl AsRef<Path>,
) -> Result<PathBuf, AssignmentError> {
    let video_path = video_path.as_ref();
    let scenes_dir = scenes_dir.as_ref();

    let playable_path = transcode::ensure_playable(video_path, &transcode_cache_dir(scenes_dir))?;

    let title = video_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("wallpaper")
        .to_string();

    let slug = sanitize_slug(&title);
    let scene_dir = scenes_dir.join(&slug);
    std::fs::create_dir_all(&scene_dir)?;

    let scene = Scene {
        format_version: 1,
        title,
        kind: SceneKind::Video {
            video: VideoLayer {
                asset: playable_path,
                r#loop: true,
                muted: true,
            },
        },
    };
    std::fs::write(
        scene_dir.join("scene.json"),
        serde_json::to_string_pretty(&scene)?,
    )?;

    Ok(scene_dir)
}

pub fn materialize_scene(
    scene: &Scene,
    scenes_dir: impl AsRef<Path>,
) -> Result<PathBuf, AssignmentError> {
    let scenes_dir = scenes_dir.as_ref();

    let scene = transcode::ensure_scene_playable(scene, &transcode_cache_dir(scenes_dir))?;
    let scene_dir = scenes_dir.join(sanitize_slug(&scene.title));
    std::fs::create_dir_all(&scene_dir)?;
    std::fs::write(
        scene_dir.join("scene.json"),
        serde_json::to_string_pretty(&scene)?,
    )?;
    Ok(scene_dir)
}

pub fn assign_scene(display_id: &str, scene_dir: impl AsRef<Path>) -> Result<(), AssignmentError> {
    let mut config = load_config()?;
    config.displays.insert(
        display_id.to_string(),
        DisplayAssignment {
            scene: scene_dir.as_ref().display().to_string(),
            muted: true,
            volume: 1.0,
            effects: EffectSettings::default(),
            playlist: None,
        },
    );
    save_config(&config)
}

pub fn update_settings(
    display_id: &str,
    muted: bool,
    volume: f32,
    effects: EffectSettings,
) -> Result<(), AssignmentError> {
    let mut config = load_config()?;
    let assignment = config
        .displays
        .get_mut(display_id)
        .ok_or_else(|| AssignmentError::NotAssigned(display_id.to_string()))?;
    assignment.muted = muted;
    assignment.volume = volume;
    assignment.effects = effects;
    save_config(&config)
}

pub fn set_global_paused(paused: bool) -> Result<(), AssignmentError> {
    let mut config = load_config()?;
    config.paused = paused;
    save_config(&config)
}

pub fn set_playlist(
    display_id: &str,
    playlist: Option<Vec<String>>,
    interval_secs: u64,
    shuffle: bool,
) -> Result<(), AssignmentError> {
    let mut config = load_config()?;
    let assignment = config
        .displays
        .get_mut(display_id)
        .ok_or_else(|| AssignmentError::NotAssigned(display_id.to_string()))?;

    match playlist {
        None => assignment.playlist = None,
        Some(entries) if entries.is_empty() => assignment.playlist = None,
        Some(entries) => {
            let first_index = if shuffle {
                fastrand_index(entries.len())
            } else {
                0
            };
            assignment.scene = entries[first_index].clone();
            assignment.playlist = Some(Playlist {
                entries,
                interval_secs: interval_secs.max(5),
                shuffle,
                current_index: first_index,
                last_advanced_at: now_unix(),
            });
        }
    }
    save_config(&config)
}

fn fastrand_index(n: usize) -> usize {
    if n <= 1 {
        return 0;
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    (seed as usize) % n
}

pub fn advance_due_playlists() -> Result<Vec<String>, AssignmentError> {
    let mut config = load_config()?;
    let now = now_unix();
    let mut advanced = Vec::new();

    for (display_id, assignment) in config.displays.iter_mut() {
        let Some(playlist) = assignment.playlist.as_mut() else {
            continue;
        };
        if playlist.entries.len() < 2 {
            continue;
        }
        if now.saturating_sub(playlist.last_advanced_at) < playlist.interval_secs {
            continue;
        }
        let next_index = if playlist.shuffle {
            let mut idx = fastrand_index(playlist.entries.len());
            if idx == playlist.current_index {
                idx = (idx + 1) % playlist.entries.len();
            }
            idx
        } else {
            (playlist.current_index + 1) % playlist.entries.len()
        };
        playlist.current_index = next_index;
        playlist.last_advanced_at = now;
        assignment.scene = playlist.entries[next_index].clone();
        advanced.push(display_id.clone());
    }

    if !advanced.is_empty() {
        save_config(&config)?;
    }
    Ok(advanced)
}

fn scene_in_use(config: &ShellConfig, scene_dir: &str) -> bool {
    config.displays.values().any(|a| {
        a.scene == scene_dir
            || a.playlist
                .as_ref()
                .is_some_and(|p| p.entries.iter().any(|e| e == scene_dir))
    })
}

#[derive(Debug, thiserror::Error)]
pub enum RemoveEntryError {
    #[error(transparent)]
    Assignment(#[from] AssignmentError),
    #[error("still assigned to a display or in a playlist — unassign it first")]
    InUse,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn remove_library_entry(scene_dir: impl AsRef<Path>) -> Result<(), RemoveEntryError> {
    let scene_dir = scene_dir.as_ref();
    let config = load_config()?;
    if scene_in_use(&config, &scene_dir.display().to_string()) {
        return Err(RemoveEntryError::InUse);
    }
    std::fs::remove_dir_all(scene_dir)?;
    Ok(())
}

pub fn list_library(scenes_dir: impl AsRef<Path>) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(scenes_dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Ok(scene) = Scene::load_dir(&path) {
            out.push((path, scene.title));
        }
    }
    out
}

pub fn thumbnail_path(scene_dir: impl AsRef<Path>) -> PathBuf {
    scene_dir.as_ref().join(".thumbnail.jpg")
}

pub fn ensure_thumbnail(scene_dir: impl AsRef<Path>) -> Result<Option<PathBuf>, AssignmentError> {
    let scene_dir = scene_dir.as_ref();
    let cached = thumbnail_path(scene_dir);
    if cached.exists() {
        return Ok(Some(cached));
    }
    if !transcode::ffmpeg_available() {
        return Ok(None);
    }
    let scene = Scene::load_dir(scene_dir)?;
    let is_video = matches!(scene.kind, SceneKind::Video { .. });

    let Some(input) = scene.asset_paths(scene_dir).into_iter().next() else {
        return Ok(None);
    };
    if generate_thumbnail(&input, &cached, is_video) {
        Ok(Some(cached))
    } else {
        Ok(None)
    }
}

fn generate_thumbnail(input: &Path, out: &Path, is_video: bool) -> bool {
    let attempt = |seek: Option<&str>| {
        let mut cmd = std::process::Command::new("ffmpeg");
        cmd.arg("-y");
        if let Some(s) = seek {
            cmd.args(["-ss", s]);
        }
        cmd.arg("-i").arg(input);
        if is_video {
            cmd.args(["-frames:v", "1"]);
        }
        cmd.args(["-vf", "scale=400:-1"]).arg(out);
        cmd.output().map(|o| o.status.success()).unwrap_or(false)
    };

    let ok = if is_video {
        attempt(Some("1")) || attempt(None)
    } else {
        attempt(None)
    };
    if !ok {
        let _ = std::fs::remove_file(out);
    }
    ok
}

pub fn transcode_cache_dir(scenes_dir: &Path) -> PathBuf {
    scenes_dir
        .parent()
        .map(|p| p.join("transcoded-cache"))
        .unwrap_or_else(|| scenes_dir.join("transcoded-cache"))
}

fn sanitize_slug(title: &str) -> String {
    let slug: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    if slug.is_empty() {
        "wallpaper".to_string()
    } else {
        slug
    }
}
