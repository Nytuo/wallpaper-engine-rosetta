use serde_json::json;
use wer_scene::we_compat::parallax_depth_of;

#[test]
fn a_real_objects_own_depth_is_used_even_when_it_is_locked() {
    let background = json!({
        "locktransforms": true,
        "parallaxDepth": "0.000 0.000",
    });
    let middle = json!({
        "locktransforms": true,
        "parallaxDepth": "0.300 0.000",
    });
    let front = json!({
        "locktransforms": true,
        "parallaxDepth": "0.480 0.000",
    });
    assert_eq!(parallax_depth_of(&background), (0.0, 0.0));
    assert_eq!(parallax_depth_of(&middle), (0.3, 0.0));
    assert_eq!(parallax_depth_of(&front), (0.48, 0.0));
}

#[test]
fn depth_is_per_axis() {
    let object = json!({ "parallaxDepth": "0.250 0.750" });
    assert_eq!(parallax_depth_of(&object), (0.25, 0.75));
}

#[test]
fn an_object_with_no_depth_falls_back_to_locktransforms() {
    assert_eq!(
        parallax_depth_of(&json!({ "locktransforms": true })),
        (0.0, 0.0)
    );
    assert_eq!(
        parallax_depth_of(&json!({ "locktransforms": false })),
        (1.0, 1.0)
    );
    assert_eq!(parallax_depth_of(&json!({})), (1.0, 1.0));
}
