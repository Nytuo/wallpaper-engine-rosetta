#ifndef WER_FFI_H
#define WER_FFI_H

#ifdef __cplusplus
extern "C" {
#endif

typedef struct EngineHandle EngineHandle;

EngineHandle *wer_engine_create(void);
void wer_engine_destroy(EngineHandle *handle);


int wer_engine_load_scene(EngineHandle *handle, const char *path);


int wer_engine_attach_metal_layer(EngineHandle *handle, void *layer, unsigned int width, unsigned int height);
int wer_engine_resize(EngineHandle *handle, unsigned int width, unsigned int height);


void wer_engine_set_parallax_mouse_position(EngineHandle *handle, float x_norm, float y_norm);


int wer_engine_needs_animation(EngineHandle *handle);


void wer_engine_set_effect_speed(EngineHandle *handle, float speed);


void wer_engine_set_occluded(EngineHandle *handle, int occluded);


int wer_engine_tick(EngineHandle *handle);


int wer_engine_tick_and_read_pixel(EngineHandle *handle, unsigned int x, unsigned int y, unsigned char *out_rgba);


char *wer_resolve_wallpaper(const char *scene_dir);


char *wer_we_import(const char *item_dir, const char *scene_dir, const char *cache_dir);

void wer_free_string(char *s);

#ifdef __cplusplus
}
#endif

#endif
