#ifndef PRISM_GHOSTTY_BRIDGE_H
#define PRISM_GHOSTTY_BRIDGE_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// Deliberately narrow proof-of-life ABI. All objects are main-thread-only.
// A runtime must outlive each surface created from it. The parent_nsview is an
// unretained NSView pointer; the surface is attached as its child.
typedef void *prism_ghostty_runtime_t;
typedef void *prism_ghostty_surface_t;

int32_t prism_ghostty_global_init(int32_t argc, char **argv);

prism_ghostty_runtime_t prism_ghostty_runtime_create(void);
void prism_ghostty_runtime_tick(prism_ghostty_runtime_t runtime);
void prism_ghostty_runtime_set_focus(prism_ghostty_runtime_t runtime,
                                     bool focused);
void prism_ghostty_runtime_destroy(prism_ghostty_runtime_t runtime);

prism_ghostty_surface_t prism_ghostty_surface_create(
    prism_ghostty_runtime_t runtime,
    void *parent_nsview,
    const char *working_directory);
void prism_ghostty_surface_set_frame(prism_ghostty_surface_t surface,
                                     double x,
                                     double y,
                                     double width,
                                     double height);
void prism_ghostty_surface_set_focus(prism_ghostty_surface_t surface,
                                     bool focused);
void prism_ghostty_surface_request_close(prism_ghostty_surface_t surface);
void prism_ghostty_surface_destroy(prism_ghostty_surface_t surface);

#ifdef __cplusplus
}
#endif

#endif
