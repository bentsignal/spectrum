#ifndef SPECTRUM_GHOSTTY_BRIDGE_H
#define SPECTRUM_GHOSTTY_BRIDGE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// Spectrum loads this ABI dynamically. Increment the version whenever a symbol,
// signature, event value, ownership rule, or geometry convention changes.
#define SPECTRUM_GHOSTTY_BRIDGE_ABI_VERSION 1
#define SPECTRUM_GHOSTTY_EVENT_TITLE 1
#define SPECTRUM_GHOSTTY_EVENT_CLOSED 2
#define SPECTRUM_GHOSTTY_EDIT_COPY 1
#define SPECTRUM_GHOSTTY_EDIT_PASTE 2

typedef void *spectrum_ghostty_runtime_t;
typedef void *spectrum_ghostty_surface_t;

typedef void (*spectrum_ghostty_event_cb)(void *userdata,
                                       uint64_t session_id,
                                       uint32_t event,
                                       const char *text,
                                       size_t text_len,
                                       bool process_alive);

uint32_t spectrum_ghostty_bridge_abi_version(void);
int32_t spectrum_ghostty_global_init(void);

spectrum_ghostty_runtime_t spectrum_ghostty_runtime_create(
    spectrum_ghostty_event_cb callback,
    void *userdata);
void spectrum_ghostty_runtime_tick(spectrum_ghostty_runtime_t runtime);
void spectrum_ghostty_runtime_set_focus(spectrum_ghostty_runtime_t runtime,
                                     bool focused);
void spectrum_ghostty_runtime_destroy(spectrum_ghostty_runtime_t runtime);

// The bridge copies cwd_utf8 and environment_json before returning. The
// environment value is a JSON object of UTF-8 string keys and values.
spectrum_ghostty_surface_t spectrum_ghostty_surface_create(
    spectrum_ghostty_runtime_t runtime,
    void *parent_nsview,
    uint64_t session_id,
    const char *cwd_utf8,
    const char *environment_json);

// Geometry is expressed in logical points with a top-left origin in the
// eframe-owned parent NSView. The Swift bridge converts it to AppKit space.
void spectrum_ghostty_surface_set_state(spectrum_ghostty_surface_t surface,
                                     double x,
                                     double y,
                                     double width,
                                     double height,
                                     bool visible,
                                     bool request_focus);
bool spectrum_ghostty_surface_edit(spectrum_ghostty_surface_t surface,
                                uint32_t action);
void spectrum_ghostty_surface_request_close(spectrum_ghostty_surface_t surface);
void spectrum_ghostty_surface_destroy(spectrum_ghostty_surface_t surface);

// Every surface must be destroyed exactly once before its runtime. The parent
// NSView is unretained and must outlive all attached surfaces. All calls and
// callbacks occur on the AppKit main thread.

#ifdef __cplusplus
}
#endif

#endif
