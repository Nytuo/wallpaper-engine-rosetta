use crate::model::Scene;
use crate::AssignmentError;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, serde::Serialize)]
pub struct CleanupSummary {
    pub removed_entries: usize,
    pub freed_bytes: u64,
}

pub fn clear_unused_workshop_cache(
    scenes_dir: impl AsRef<Path>,
    workshop_downloads_dir: impl AsRef<Path>,
    transcode_cache_dir: impl AsRef<Path>,
) -> Result<CleanupSummary, AssignmentError> {
    let used = referenced_asset_paths(scenes_dir.as_ref())?;

    let mut summary = CleanupSummary::default();
    remove_unreferenced(workshop_downloads_dir.as_ref(), &used, &mut summary)?;
    remove_unreferenced(transcode_cache_dir.as_ref(), &used, &mut summary)?;
    Ok(summary)
}

fn referenced_asset_paths(scenes_dir: &Path) -> Result<HashSet<PathBuf>, AssignmentError> {
    let mut used = HashSet::new();
    let Ok(entries) = std::fs::read_dir(scenes_dir) else {
        return Ok(used);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Ok(scene) = Scene::load_dir(&path) {
            used.extend(scene.asset_paths(&path));
        }
    }
    Ok(used)
}

fn remove_unreferenced(
    dir: &Path,
    used: &HashSet<PathBuf>,
    summary: &mut CleanupSummary,
) -> Result<(), AssignmentError> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let in_use = used.contains(&path) || used.iter().any(|u| u.starts_with(&path));
        if in_use {
            continue;
        }
        let size = dir_size(&path);
        if path.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
        summary.removed_entries += 1;
        summary.freed_bytes += size;
    }
    Ok(())
}

fn dir_size(path: &Path) -> u64 {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.is_file() {
            return meta.len();
        }
    }
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            total += dir_size(&entry.path());
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SceneKind, VideoLayer};

    fn write_scene(scene_dir: &Path, asset: &Path) {
        std::fs::create_dir_all(scene_dir).unwrap();
        let scene = Scene {
            format_version: 1,
            title: "test".into(),
            kind: SceneKind::Video {
                video: VideoLayer {
                    asset: asset.to_path_buf(),
                    r#loop: true,
                    muted: true,
                },
            },
        };
        std::fs::write(
            scene_dir.join("scene.json"),
            serde_json::to_string(&scene).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn keeps_referenced_entries_and_removes_orphans() {
        let tmp = std::env::temp_dir().join(format!("wer-cleanup-test-{}", std::process::id()));
        let scenes_dir = tmp.join("scenes");
        let workshop_dir = tmp.join("workshop_downloads");
        let cache_dir = tmp.join("transcoded-cache");
        let _ = std::fs::remove_dir_all(&tmp);

        std::fs::create_dir_all(workshop_dir.join("in_use_item")).unwrap();
        std::fs::write(workshop_dir.join("in_use_item").join("video.mp4"), b"data").unwrap();
        write_scene(
            &scenes_dir.join("active"),
            &workshop_dir.join("in_use_item").join("video.mp4"),
        );

        std::fs::create_dir_all(workshop_dir.join("orphaned_item").join(".DepotDownloader"))
            .unwrap();

        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::write(cache_dir.join("in-use.mp4"), b"data").unwrap();
        std::fs::write(cache_dir.join("orphaned.mp4"), b"stale data").unwrap();
        write_scene(&scenes_dir.join("another"), &cache_dir.join("in-use.mp4"));

        let summary = clear_unused_workshop_cache(&scenes_dir, &workshop_dir, &cache_dir).unwrap();

        assert!(
            workshop_dir.join("in_use_item").exists(),
            "in-use download should survive"
        );
        assert!(
            !workshop_dir.join("orphaned_item").exists(),
            "orphaned download should be removed"
        );
        assert!(
            cache_dir.join("in-use.mp4").exists(),
            "in-use cache file should survive"
        );
        assert!(
            !cache_dir.join("orphaned.mp4").exists(),
            "orphaned cache file should be removed"
        );
        assert_eq!(summary.removed_entries, 2);

        std::fs::remove_dir_all(&tmp).ok();
    }
}
