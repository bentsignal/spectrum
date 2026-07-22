#ifndef SPECTRUM_GHOSTTY_BRIDGE_H
#define SPECTRUM_GHOSTTY_BRIDGE_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// Deliberately narrow proof-of-life ABI. All objects are main-thread-only.
// A runtime must outlive each surface created from it. The parent_nsview is an
// unretained NSView pointer; the surface is attached as its child.
typedef void *spectrum_ghostty_runtime_t;
typedef void *spectrum_ghostty_surface_t;

int32_t spectrum_ghostty_global_init(int32_t argc, char **argv);

spectrum_ghostty_runtime_t spectrum_ghostty_runtime_create(void);
void spectrum_ghostty_runtime_tick(spectrum_ghostty_runtime_t runtime);
void spectrum_ghostty_runtime_set_focus(spectrum_ghostty_runtime_t runtime,
                                     bool focused);
void spectrum_ghostty_runtime_destroy(spectrum_ghostty_runtime_t runtime);

spectrum_ghostty_surface_t spectrum_ghostty_surface_create(
    spectrum_ghostty_runtime_t runtime,
    void *parent_nsview,
    const char *working_directory);
void spectrum_ghostty_surface_set_frame(spectrum_ghostty_surface_t surface,
                                     double x,
                                     double y,
                                     double width,
                                     double height);
void spectrum_ghostty_surface_set_focus(spectrum_ghostty_surface_t surface,
                                     bool focused);
void spectrum_ghostty_surface_request_close(spectrum_ghostty_surface_t surface);
void spectrum_ghostty_surface_destroy(spectrum_ghostty_surface_t surface);

#ifdef __cplusplus
}
#endif

#endif
