use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VideoError {
    #[error("no decode backend available for {0}")]
    NoBackend(PathBuf),
    #[error("failed to open {path}: {reason}")]
    OpenFailed { path: PathBuf, reason: String },
    #[error("decode backend not yet implemented on this platform")]
    Unimplemented,
}

#[derive(Debug, Clone, Copy)]
pub struct GpuTextureHandle(pub u64);

pub struct VideoInfo {
    pub width: u32,
    pub height: u32,

    pub duration_secs: f64,
}

pub trait VideoSource: Send {
    fn info(&self) -> VideoInfo;

    fn next_frame(&mut self) -> Option<GpuTextureHandle>;

    fn set_muted(&mut self, muted: bool);
    fn set_paused(&mut self, paused: bool);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Auto,
    #[cfg(target_os = "macos")]
    VideoToolbox,
    #[cfg(target_os = "windows")]
    MediaFoundation,
    FfmpegSoftware,
}

pub fn open(path: impl AsRef<Path>, _backend: Backend) -> Result<Box<dyn VideoSource>, VideoError> {
    Err(VideoError::NoBackend(path.as_ref().to_path_buf()))
}
