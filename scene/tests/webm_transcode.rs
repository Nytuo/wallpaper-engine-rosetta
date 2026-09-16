use std::process::Command;

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn transcodes_a_real_webm_to_playable_mp4() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not available, skipping");
        return;
    }

    let tmp = std::env::temp_dir().join(format!("wer-webm-test-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let webm_path = tmp.join("source.webm");
    let cache_dir = tmp.join("cache");

    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30:duration=1",
        ])
        .args(["-c:v", "libvpx-vp9", "-b:v", "500k"])
        .arg(&webm_path)
        .status()
        .expect("failed to generate test webm");
    assert!(status.success(), "test fixture generation failed");

    let result =
        wer_scene::transcode::ensure_playable(&webm_path, &cache_dir).expect("transcode failed");
    assert_ne!(
        result, webm_path,
        "should return a different (transcoded) path for webm"
    );
    assert!(result.exists(), "transcoded file should exist");
    assert_eq!(result.extension().unwrap(), "mp4");

    let second = wer_scene::transcode::ensure_playable(&webm_path, &cache_dir)
        .expect("cached transcode failed");
    assert_eq!(result, second);

    std::fs::remove_dir_all(&tmp).ok();
}
