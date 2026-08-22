/**
 * halcyon.h — C API for Halcyon eBPF Process Monitor
 *
 * This header provides FFI bindings for the Halcyon eBPF process monitor,
 * allowing integration with C, C++, Python, Go, and other languages.
 *
 * Thread safety: All functions are thread-safe unless noted otherwise.
 * Memory: Returned strings must be freed with halcyon_free_string().
 */

#ifndef HALCYON_H
#define HALCYON_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── Version ────────────────────────────────────────────────────────────── */

#define HALCYON_VERSION_MAJOR 0
#define HALCYON_VERSION_MINOR 3
#define HALCYON_VERSION_PATCH 0

/**
 * Returns the library version as a string (e.g., "0.3.0").
 * The returned string is static and must NOT be freed.
 */
const char* halcyon_version(void);

/* ── Opaque handles ─────────────────────────────────────────────────────── */

/** Opaque handle to a Halcyon monitor instance. */
typedef struct halcyon_monitor halcyon_monitor_t;

/** Opaque handle to a Halcyon event iterator. */
typedef struct halcyon_event_iter halcyon_event_iter_t;

/* ── Event types ────────────────────────────────────────────────────────── */

/** Event kind enum. */
typedef enum {
    HALCYON_EVENT_EXEC = 0,
    HALCYON_EVENT_OPEN = 1,
} halcyon_event_kind_t;

/** A single process event. */
typedef struct {
    halcyon_event_kind_t kind;
    uint32_t pid;
    uint32_t uid;
    const char* comm;        /**< Process name (null-terminated). */
    const char* file;        /**< File path (for OPEN events, NULL for EXEC). */
    const char* argv;        /**< Command line (for EXEC events, NULL for OPEN). */
    const char* timestamp;   /**< ISO timestamp string. */
} halcyon_event_t;

/** Process statistics. */
typedef struct {
    uint32_t pid;
    uint32_t ppid;
    const char* comm;
    uint64_t total_opens;
    uint64_t total_execs;
    uint64_t alerts;
    uint64_t window_opens;   /**< Opens in the current sliding window. */
} halcyon_process_stats_t;

/** File access ranking. */
typedef struct {
    const char* path;
    uint64_t count;
    const char* extension;
    double entropy;          /**< Shannon entropy (0.0–1.0). */
} halcyon_file_rank_t;

/** Alert record. */
typedef struct {
    uint32_t pid;
    uint32_t uid;
    const char* comm;
    uint64_t opens;
    const char* timestamp;
} halcyon_alert_t;

/** Monitor statistics summary. */
typedef struct {
    uint64_t total_events;
    uint64_t total_lost;
    uint64_t uptime_secs;
    uint64_t active_pids;
    uint64_t threshold;
} halcyon_stats_t;

/* ── Error handling ─────────────────────────────────────────────────────── */

/** Error codes. */
typedef enum {
    HALCYON_OK = 0,
    HALCYON_ERR_NOMEM = -1,
    HALCYON_ERR_INVAL = -2,
    HALCYON_ERR_PERM  = -3,
    HALCYON_ERR_IO    = -4,
    HALCYON_ERR_NOT_FOUND = -5,
} halcyon_err_t;

/**
 * Returns a human-readable error message for an error code.
 * The returned string is static and must NOT be freed.
 */
const char* halcyon_strerror(halcyon_err_t err);

/* ── Monitor lifecycle ──────────────────────────────────────────────────── */

/**
 * Creates a new Halcyon monitor instance.
 *
 * @param bpf_path     Path to the compiled eBPF object file.
 * @param threshold    Alert threshold (files opened per second).
 * @param out          Pointer to receive the monitor handle.
 * @return HALCYON_OK on success, or an error code.
 */
halcyon_err_t halcyon_monitor_create(
    const char* bpf_path,
    uint64_t threshold,
    halcyon_monitor_t** out
);

/**
 * Destroys a monitor instance and frees all resources.
 */
void halcyon_monitor_destroy(halcyon_monitor_t* monitor);

/**
 * Polls for new events. Returns events since the last poll.
 *
 * @param monitor   The monitor instance.
 * @param events    Array to receive events (caller-allocated).
 * @param max_events Maximum number of events to return.
 * @param count     Output: number of events written.
 * @return HALCYON_OK on success.
 */
halcyon_err_t halcyon_monitor_poll(
    halcyon_monitor_t* monitor,
    halcyon_event_t* events,
    uint32_t max_events,
    uint32_t* count
);

/**
 * Returns current monitor statistics.
 */
halcyon_err_t halcyon_monitor_stats(
    halcyon_monitor_t* monitor,
    halcyon_stats_t* stats
);

/**
 * Updates the alert threshold at runtime.
 */
halcyon_err_t halcyon_monitor_set_threshold(
    halcyon_monitor_t* monitor,
    uint64_t threshold
);

/* ── Process queries ────────────────────────────────────────────────────── */

/**
 * Returns all tracked processes, sorted by window opens (descending).
 *
 * @param monitor   The monitor instance.
 * @param stats     Array to receive process stats.
 * @param max_stats Maximum number of processes to return.
 * @param count     Output: number of processes written.
 */
halcyon_err_t halcyon_monitor_processes(
    halcyon_monitor_t* monitor,
    halcyon_process_stats_t* stats,
    uint32_t max_stats,
    uint32_t* count
);

/**
 * Returns top-N most-opened files.
 *
 * @param monitor   The monitor instance.
 * @param files     Array to receive file ranks.
 * @param n         Maximum number of files to return.
 * @param count     Output: number of files written.
 */
halcyon_err_t halcyon_monitor_top_files(
    halcyon_monitor_t* monitor,
    halcyon_file_rank_t* files,
    uint32_t n,
    uint32_t* count
);

/* ── Memory management ──────────────────────────────────────────────────── */

/**
 * Frees a string returned by the Halcyon API.
 * Safe to call with NULL.
 */
void halcyon_free_string(char* s);

/**
 * Frees an array of events returned by halcyon_monitor_poll.
 */
void halcyon_free_events(halcyon_event_t* events, uint32_t count);

/**
 * Frees an array of process stats.
 */
void halcyon_free_processes(halcyon_process_stats_t* stats, uint32_t count);

/**
 * Frees an array of file ranks.
 */
void halcyon_free_files(halcyon_file_rank_t* files, uint32_t count);

/* ── Convenience macros ─────────────────────────────────────────────────── */

/** Check if a halcyon error code indicates success. */
#define HALCYON_OK(err) ((err) == HALCYON_OK)

/** Get the last error message (thread-local). */
const char* halcyon_last_error(void);

#ifdef __cplusplus
}
#endif

#endif /* HALCYON_H */
