use crate::model::{LayerContent, Scene, SceneKind};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum TranscodeError {
    #[error(
        "\"{0}\" is a WebM file, which macOS's video framework can't play natively (confirmed: no WebM/VP8/VP9/Opus support at all). \
         Converting it needs ffmpeg, which isn't installed — install it (e.g. `brew install ffmpeg`) and try again, \
         or use an MP4/MOV/M4V file instead."
    )]
    FfmpegNotFound(PathBuf),
    #[error("ffmpeg failed converting \"{0}\" (exit status {1:?}) — see stderr below:\n{2}")]
    FfmpegFailed(PathBuf, Option<i32>, String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

fn is_webm(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("webm"))
        .unwrap_or(false)
}

pub(crate) fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn ensure_playable(video_path: &Path, cache_dir: &Path) -> Result<PathBuf, TranscodeError> {
    if !is_webm(video_path) {
        return Ok(video_path.to_path_buf());
    }

    std::fs::create_dir_all(cache_dir)?;
    let cached = cache_dir.join(format!("{}.mp4", cache_key(video_path)));
    if cached.exists() {
        return Ok(cached);
    }

    if !ffmpeg_available() {
        return Err(TranscodeError::FfmpegNotFound(video_path.to_path_buf()));
    }

    let output = Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(video_path)
        .args([
            "-c:v", "libx264", "-preset", "fast", "-crf", "20", "-pix_fmt", "yuv420p",
        ])
        .args(["-c:a", "aac", "-b:a", "128k"])
        .arg("-movflags")
        .arg("+faststart")
        .arg(&cached)
        .output()?;

    if !output.status.success() {
        let _ = std::fs::remove_file(&cached);
        return Err(TranscodeError::FfmpegFailed(
            video_path.to_path_buf(),
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }

    Ok(cached)
}

pub fn ensure_scene_playable(scene: &Scene, cache_dir: &Path) -> Result<Scene, TranscodeError> {
    let mut scene = scene.clone();
    match &mut scene.kind {
        SceneKind::Video { video } => {
            video.asset = ensure_playable(&video.asset, cache_dir)?;
        }
        SceneKind::Layered { layers, .. } => {
            for layer in layers {
                if let LayerContent::Video { asset } = &mut layer.content {
                    *asset = ensure_playable(asset, cache_dir)?;
                }
            }
        }
    }
    Ok(scene)
}

fn cache_key(video_path: &Path) -> String {
    let meta = std::fs::metadata(video_path).ok();
    let stem = video_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video");
    let slug: String = stem
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let fingerprint = meta
        .map(|m| {
            let len = m.len();
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("{len}-{mtime}")
        })
        .unwrap_or_else(|| "0-0".to_string());
    format!("{slug}-{fingerprint}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_webm_passes_through_untouched() {
        let path = Path::new("/some/video.mp4");
        assert_eq!(
            ensure_playable(path, Path::new("/tmp/does-not-matter")).unwrap(),
            path
        );
    }
}
