use std::fs;
use wer_scene::model::SceneKind;
use wer_scene::we_compat::{import_dir, WeCompatError};

#[test]
fn video_type_is_recognized_regardless_of_capitalization() {
    let tmp = std::env::temp_dir().join(format!("wer-we-video-case-test-{}", std::process::id()));
    fs::create_dir_all(&tmp).unwrap();

    fs::write(
        tmp.join("project.json"),
        r#"{"title": "Some Wallpaper", "type": "Video", "file": "bg.mp4"}"#,
    )
    .unwrap();
    fs::write(
        tmp.join("bg.mp4"),
        b"not a real video, just bytes for this test",
    )
    .unwrap();

    let result = import_dir(&tmp).expect("capitalized \"Video\" type should still be recognized");
    assert_eq!(result.scene.title, "Some Wallpaper");

    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn packed_scene_with_no_translatable_layers_reports_a_specific_reason() {
    let tmp = std::env::temp_dir().join(format!("wer-we-empty-scene-test-{}", std::process::id()));
    fs::create_dir_all(&tmp).unwrap();
    fs::write(
        tmp.join("project.json"),
        r#"{"title": "Empty Scene", "type": "Scene", "file": "scene.json"}"#,
    )
    .unwrap();
    fs::write(
        tmp.join("scene.pkg"),
        build_pkg(&[("scene.json", br#"{"objects":[]}"#)]),
    )
    .unwrap();

    let err = import_dir(&tmp)
        .expect_err("a scene with zero translatable layers should not silently succeed");
    match err {
        WeCompatError::UnsupportedPackedScene { title } => assert_eq!(title, "Empty Scene"),
        other => panic!("expected UnsupportedPackedScene, got: {other}"),
    }

    fs::remove_dir_all(&tmp).ok();
}

fn build_pkg(entries: &[(&str, &[u8])]) -> Vec<u8> {
    fn write_string(out: &mut Vec<u8>, s: &str) {
        out.extend((s.len() as i32).to_le_bytes());
        out.extend(s.as_bytes());
    }
    let mut out = Vec::new();
    write_string(&mut out, "PKGV0020");
    out.extend((entries.len() as i32).to_le_bytes());
    let mut data = Vec::new();
    for (path, content) in entries {
        write_string(&mut out, path);
        out.extend((data.len() as i32).to_le_bytes());
        out.extend((content.len() as i32).to_le_bytes());
        data.extend_from_slice(content);
    }
    out.extend(data);
    out
}
