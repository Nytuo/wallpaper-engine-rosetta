use std::ffi::{c_char, CStr, CString};
use std::os::raw::c_int;
use wer_engine::Engine;

pub struct EngineHandle(Engine);

#[no_mangle]
pub extern "C" fn wer_engine_create() -> *mut EngineHandle {
    Box::into_raw(Box::new(EngineHandle(Engine::new())))
}

#[no_mangle]
pub extern "C" fn wer_engine_destroy(handle: *mut EngineHandle) {
    if handle.is_null() {
        return;
    }
    drop(unsafe { Box::from_raw(handle) });
}

#[no_mangle]
pub extern "C" fn wer_engine_load_scene(handle: *mut EngineHandle, path: *const c_char) -> c_int {
    if handle.is_null() || path.is_null() {
        return -1;
    }
    let handle = unsafe { &mut *handle };
    let path = match unsafe { CStr::from_ptr(path) }.to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };
    match wer_scene::Scene::load_dir(path) {
        Ok(scene) => match handle.0.load_scene(scene, path) {
            Ok(()) => 0,
            Err(err) => {
                tracing::error!("failed to load scene's GPU layer resources at {path}: {err}");
                -1
            }
        },
        Err(err) => {
            tracing::error!("failed to load scene at {path}: {err}");
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn wer_engine_attach_metal_layer(
    handle: *mut EngineHandle,
    layer: *mut std::ffi::c_void,
    width: u32,
    height: u32,
) -> c_int {
    if handle.is_null() || layer.is_null() {
        return -1;
    }
    let handle = unsafe { &mut *handle };
    match unsafe { handle.0.attach_metal_layer(layer, width, height) } {
        Ok(()) => 0,
        Err(err) => {
            tracing::error!("failed to attach Metal layer: {err}");
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn wer_engine_resize(handle: *mut EngineHandle, width: u32, height: u32) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let handle = unsafe { &mut *handle };
    match handle.0.resize(width, height) {
        Ok(()) => 0,
        Err(err) => {
            tracing::error!("failed to resize engine surface: {err}");
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn wer_engine_set_parallax_mouse_position(
    handle: *mut EngineHandle,
    x_norm: f32,
    y_norm: f32,
) {
    if handle.is_null() {
        return;
    }
    let handle = unsafe { &mut *handle };
    handle.0.set_parallax_mouse_position(x_norm, y_norm);
}

#[no_mangle]
pub extern "C" fn wer_engine_set_effect_speed(handle: *mut EngineHandle, speed: f32) {
    if handle.is_null() {
        return;
    }
    let handle = unsafe { &mut *handle };
    handle.0.set_effect_speed(speed);
}

#[no_mangle]
pub extern "C" fn wer_engine_tick_and_read_pixel(
    handle: *mut EngineHandle,
    x: u32,
    y: u32,
    out_rgba: *mut u8,
) -> c_int {
    if handle.is_null() || out_rgba.is_null() {
        return -1;
    }
    let handle = unsafe { &mut *handle };
    match handle.0.tick_and_read_pixel(x, y) {
        Ok(pixel) => {
            unsafe { std::ptr::copy_nonoverlapping(pixel.as_ptr(), out_rgba, 4) };
            0
        }
        Err(err) => {
            tracing::error!("tick_and_read_pixel failed: {err}");
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn wer_engine_set_occluded(handle: *mut EngineHandle, occluded: c_int) {
    if handle.is_null() {
        return;
    }
    let handle = unsafe { &mut *handle };
    handle.0.set_occluded(occluded != 0);
}

#[no_mangle]
pub extern "C" fn wer_engine_tick(handle: *mut EngineHandle) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let handle = unsafe { &mut *handle };
    match handle.0.tick() {
        Ok(()) => 0,
        Err(err) => {
            tracing::error!("engine tick failed: {err}");
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn wer_engine_needs_animation(handle: *mut EngineHandle) -> c_int {
    if handle.is_null() {
        return 0;
    }
    let handle = unsafe { &*handle };
    handle.0.needs_animation() as c_int
}

#[no_mangle]
pub extern "C" fn wer_resolve_wallpaper(scene_dir: *const c_char) -> *mut c_char {
    if scene_dir.is_null() {
        return std::ptr::null_mut();
    }
    let scene_dir = match unsafe { CStr::from_ptr(scene_dir) }.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    let dir = std::path::Path::new(scene_dir);

    let scene = match wer_scene::Scene::load_dir(dir) {
        Ok(scene) => scene,
        Err(err) => {
            tracing::error!("failed to resolve wallpaper at {scene_dir}: {err}");
            return std::ptr::null_mut();
        }
    };

    let json = match &scene.kind {
        wer_scene::model::SceneKind::Video { .. } => {
            let asset = scene
                .primary_video_path(dir)
                .expect("Video kind always has a primary asset");
            serde_json::json!({
                "kind": "video",
                "title": scene.title,
                "asset": asset.display().to_string(),
                "muted": scene.is_muted(),
            })
        }
        wer_scene::model::SceneKind::Layered {
            canvas_width,
            canvas_height,
            parallax,
            layers,
        } => {
            let layer_json: Vec<serde_json::Value> = layers
                .iter()
                .filter_map(|layer| {
                    let (kind, asset, volume) = match &layer.content {
                        wer_scene::model::LayerContent::Video { asset } => ("video", asset, 1.0),
                        wer_scene::model::LayerContent::Image { asset } => ("image", asset, 1.0),
                        wer_scene::model::LayerContent::Shader { asset } => ("shader", asset, 1.0),
                        wer_scene::model::LayerContent::Audio { asset, volume } => {
                            ("audio", asset, *volume)
                        }

                        wer_scene::model::LayerContent::Particles(_) => return None,
                    };
                    Some(serde_json::json!({
                        "kind": kind,
                        "asset": asset.display().to_string(),
                        "x": layer.transform.x,
                        "y": layer.transform.y,
                        "width": layer.transform.width,
                        "height": layer.transform.height,
                        "rotation": layer.transform.rotation,
                        "parallax_depth": layer.parallax_depth,
                        "parallax_depth_y": layer.parallax_depth_y,
                        "volume": volume,
                    }))
                })
                .collect();
            serde_json::json!({
                "kind": "layered",
                "title": scene.title,
                "canvas_width": canvas_width,
                "canvas_height": canvas_height,
                "parallax_enabled": parallax.enabled,
                "parallax_amount": parallax.amount,
                "parallax_mouse_influence": parallax.mouse_influence,
                "layers": layer_json,
            })
        }
    };

    match CString::new(json.to_string()) {
        Ok(cstring) => cstring.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn wer_we_import(
    item_dir: *const c_char,
    scene_dir: *const c_char,
    cache_dir: *const c_char,
) -> *mut c_char {
    fn arg<'a>(ptr: *const c_char) -> Option<&'a str> {
        if ptr.is_null() {
            return None;
        }
        unsafe { CStr::from_ptr(ptr) }.to_str().ok()
    }
    let (Some(item_dir), Some(scene_dir), Some(cache_dir)) =
        (arg(item_dir), arg(scene_dir), arg(cache_dir))
    else {
        return std::ptr::null_mut();
    };

    let result = (|| -> Result<serde_json::Value, String> {
        let imported = wer_scene::we_compat::import_dir(item_dir).map_err(|e| e.to_string())?;
        let scene = wer_scene::transcode::ensure_scene_playable(
            &imported.scene,
            std::path::Path::new(cache_dir),
        )
        .map_err(|e| e.to_string())?;
        std::fs::create_dir_all(scene_dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(&scene).map_err(|e| e.to_string())?;
        std::fs::write(std::path::Path::new(scene_dir).join("scene.json"), json)
            .map_err(|e| e.to_string())?;
        let (kind, width, height) = match scene.canvas_size() {
            Some((w, h)) => ("layered", w, h),
            None => ("video", 0.0, 0.0),
        };
        Ok(serde_json::json!({
            "ok": true,
            "title": scene.title,
            "kind": kind,
            "canvas_width": width,
            "canvas_height": height,
            "skipped": imported.skipped,
        }))
    })();

    let json = result.unwrap_or_else(|error| serde_json::json!({ "ok": false, "error": error }));
    match CString::new(json.to_string()) {
        Ok(cstring) => cstring.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn wer_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    drop(unsafe { CString::from_raw(s) });
}
