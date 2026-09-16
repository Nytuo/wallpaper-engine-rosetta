use std::fs;
use wer_scene::assignment;

fn write_video_scene(dir: &std::path::Path, title: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("scene.json"),
        format!(
            r#"{{"format_version":1,"title":"{title}","kind":{{"Video":{{"video":{{"asset":"/tmp/{title}.mp4","loop":true,"muted":true}}}}}}}}"#
        ),
    )
    .unwrap();
}

#[test]
fn playlist_rotation_and_entry_removal_end_to_end() {
    let tmp = std::env::temp_dir().join(format!("wer-playlist-test-{}", std::process::id()));
    let home = tmp.join("home");
    fs::create_dir_all(&home).unwrap();

    unsafe { std::env::set_var("HOME", &home) };

    let scenes_dir = home.join("scenes");
    for name in ["a", "b", "c", "d"] {
        write_video_scene(&scenes_dir.join(name), name);
    }
    let path_of = |name: &str| scenes_dir.join(name).display().to_string();
    let (a, b, c, d) = (path_of("a"), path_of("b"), path_of("c"), path_of("d"));

    assignment::assign_scene("disp1", &a).unwrap();

    assignment::set_playlist(
        "disp1",
        Some(vec![a.clone(), b.clone(), c.clone()]),
        1,
        false,
    )
    .unwrap();
    let cfg = assignment::load_config().unwrap();
    let assignment_after_set = cfg.displays.get("disp1").unwrap();
    assert_eq!(
        assignment_after_set.scene, a,
        "sequential mode activates the first entry"
    );
    let playlist = assignment_after_set
        .playlist
        .as_ref()
        .expect("playlist should be set");
    assert_eq!(playlist.entries, vec![a.clone(), b.clone(), c.clone()]);
    assert_eq!(
        playlist.interval_secs, 5,
        "a below-floor interval is clamped to the 5s minimum"
    );
    assert!(!playlist.shuffle);

    let advanced = assignment::advance_due_playlists().unwrap();
    assert!(
        advanced.is_empty(),
        "should not rotate before its interval elapses"
    );

    let mut cfg = assignment::load_config().unwrap();
    cfg.displays
        .get_mut("disp1")
        .unwrap()
        .playlist
        .as_mut()
        .unwrap()
        .last_advanced_at = 0;
    assignment::save_config(&cfg).unwrap();

    let advanced = assignment::advance_due_playlists().unwrap();
    assert_eq!(advanced, vec!["disp1".to_string()]);
    let cfg = assignment::load_config().unwrap();
    let after = cfg.displays.get("disp1").unwrap();
    assert_eq!(
        after.scene, b,
        "sequential rotation moves to the next entry"
    );
    assert_eq!(after.playlist.as_ref().unwrap().current_index, 1);

    assert!(
        assignment::remove_library_entry(&c).is_err(),
        "in-use entries must not be deletable"
    );
    assert!(scenes_dir.join("c").exists());

    assert!(assignment::remove_library_entry(&d).is_ok());
    assert!(!scenes_dir.join("d").exists());

    assignment::set_playlist("disp1", None, 5, false).unwrap();
    let cfg = assignment::load_config().unwrap();
    let after_off = cfg.displays.get("disp1").unwrap();
    assert_eq!(after_off.scene, b);
    assert!(after_off.playlist.is_none());

    assert!(assignment::remove_library_entry(&b).is_err());

    assignment::assign_scene("disp1", &a).unwrap();
    assert!(assignment::remove_library_entry(&b).is_ok());

    let _ = fs::remove_dir_all(&tmp);
}
