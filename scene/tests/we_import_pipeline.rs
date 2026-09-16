use std::fs;
use wer_scene::{assignment, we_compat};

#[test]
fn imports_a_downloaded_we_video_item_end_to_end() {
    let tmp = std::env::temp_dir().join(format!("wer-we-import-test-{}", std::process::id()));
    let downloaded = tmp.join("downloaded_item");
    let scenes_dir = tmp.join("scenes");
    fs::create_dir_all(&downloaded).unwrap();

    fs::write(
        downloaded.join("project.json"),
        r#"{"title": "Fake WE Video Item", "type": "video", "file": "bg.mp4"}"#,
    )
    .unwrap();
    fs::write(
        downloaded.join("bg.mp4"),
        b"not a real video, just bytes for this test",
    )
    .unwrap();

    let result = we_compat::import_dir(&downloaded)
        .expect("import_dir should parse a video-type project.json");
    assert_eq!(result.scene.title, "Fake WE Video Item");
    assert!(result.skipped.is_empty());

    let scene_dir = assignment::materialize_scene(&result.scene, &scenes_dir)
        .expect("materialize_scene failed");
    assert!(scene_dir.join("scene.json").exists());

    let reloaded = wer_scene::Scene::load_dir(&scene_dir)
        .expect("reloading the materialized scene should work");
    let video_path = reloaded
        .primary_video_path(&scene_dir)
        .expect("should be a video scene");
    assert_eq!(video_path, downloaded.join("bg.mp4"));

    fs::remove_dir_all(&tmp).ok();
}
