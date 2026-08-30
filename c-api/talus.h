/**
 * talus.h — C API for Talus eBPF Process Monitor
 *
 * This header provides FFI bindings for the Talus eBPF process monitor,
 * allowing integration with C, C++, Python, Go, and other languages.
 *
 * Thread safety: All functions are thread-safe unless noted otherwise.
 * Memory: Returned strings must be freed with talus_free_string().
 */

#ifndef TALUS_H
#define TALUS_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── Version ────────────────────────────────────────────────────────────── */

#define TALUS_VERSION_MAJOR 0
#define TALUS_VERSION_MINOR 7
#define TALUS_VERSION_PATCH 0

/**
 * Returns the library version as a string (e.g., "0.3.0").
 * The returned string is static and must NOT be freed.
 */
const char* talus_version(void);

/* ── Opaque handles ─────────────────────────────────────────────────────── */

/** Opaque handle to a Talus monitor instance. */
typedef struct talus_monitor talus_monitor_t;

/** Opaque handle to a Talus event iterator. */
typedef struct talus_event_iter talus_event_iter_t;

/* ── Event types ────────────────────────────────────────────────────────── */

/** Event kind enum. */
typedef enum {
    TALUS_EVENT_EXEC = 0,
    TALUS_EVENT_OPEN = 1,
} talus_event_kind_t;

/** A single process event. */
typedef struct {
    talus_event_kind_t kind;
    uint32_t pid;
    uint32_t uid;
    const char* comm;        /**< Process name (null-terminated). */
    const char* file;        /**< File path (for OPEN events, NULL for EXEC). */
    const char* argv;        /**< Command line (for EXEC events, NULL for OPEN). */
    const char* timestamp;   /**< ISO timestamp string. */
} talus_event_t;

/** Process statistics. */
typedef struct {
    uint32_t pid;
    uint32_t ppid;
    const char* comm;
    uint64_t total_opens;
    uint64_t total_execs;
    uint64_t alerts;
    uint64_t window_opens;   /**< Opens in the current sliding window. */
} talus_process_stats_t;

/** File access ranking. */
typedef struct {
    const char* path;
    uint64_t count;
    const char* extension;
    double entropy;          /**< Shannon entropy (0.0–1.0). */
} talus_file_rank_t;

/** Alert record. */
typedef struct {
    uint32_t pid;
    uint32_t uid;
    const char* comm;
    uint64_t opens;
    const char* timestamp;
} talus_alert_t;

/** Monitor statistics summary. */
typedef struct {
    uint64_t total_events;
    uint64_t total_lost;
    uint64_t uptime_secs;
    uint64_t active_pids;
    uint64_t threshold;
} talus_stats_t;

/* ── Error handling ─────────────────────────────────────────────────────── */

/** Error codes. */
typedef enum {
    TALUS_OK = 0,
    TALUS_ERR_NOMEM = -1,
    TALUS_ERR_INVAL = -2,
    TALUS_ERR_PERM  = -3,
    TALUS_ERR_IO    = -4,
    TALUS_ERR_NOT_FOUND = -5,
} talus_err_t;

/**
 * Returns a human-readable error message for an error code.
 * The returned string is static and must NOT be freed.
 */
const char* talus_strerror(talus_err_t err);

/* ── Monitor lifecycle ──────────────────────────────────────────────────── */

/**
 * Creates a new Talus monitor instance.
 *
 * @param bpf_path     Path to the compiled eBPF object file.
 * @param threshold    Alert threshold (files opened per second).
 * @param out          Pointer to receive the monitor handle.
 * @return TALUS_OK on success, or an error code.
 */
talus_err_t talus_monitor_create(
    const char* bpf_path,
    uint64_t threshold,
    talus_monitor_t** out
);

/**
 * Destroys a monitor instance and frees all resources.
 */
void talus_monitor_destroy(talus_monitor_t* monitor);

/**
 * Polls for new events. Returns events since the last poll.
 *
 * @param monitor   The monitor instance.
 * @param events    Array to receive events (caller-allocated).
 * @param max_events Maximum number of events to return.
 * @param count     Output: number of events written.
 * @return TALUS_OK on success.
 */
talus_err_t talus_monitor_poll(
    talus_monitor_t* monitor,
    talus_event_t* events,
    uint32_t max_events,
    uint32_t* count
);

/**
 * Returns current monitor statistics.
 */
talus_err_t talus_monitor_stats(
    talus_monitor_t* monitor,
    talus_stats_t* stats
);

/**
 * Updates the alert threshold at runtime.
 */
talus_err_t talus_monitor_set_threshold(
    talus_monitor_t* monitor,
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
talus_err_t talus_monitor_processes(
    talus_monitor_t* monitor,
    talus_process_stats_t* stats,
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
talus_err_t talus_monitor_top_files(
    talus_monitor_t* monitor,
    talus_file_rank_t* files,
    uint32_t n,
    uint32_t* count
);

/* ── Memory management ──────────────────────────────────────────────────── */

/**
 * Frees a string returned by the Talus API.
 * Safe to call with NULL.
 */
void talus_free_string(char* s);

/**
 * Frees an array of events returned by talus_monitor_poll.
 */
void talus_free_events(talus_event_t* events, uint32_t count);

/**
 * Frees an array of process stats.
 */
void talus_free_processes(talus_process_stats_t* stats, uint32_t count);

/**
 * Frees an array of file ranks.
 */
void talus_free_files(talus_file_rank_t* files, uint32_t count);

/* ── License API ───────────────────────────────────────────────────────── */

/**
 * Returns the current license tier as a string.
 * Possible values: "community", "enterprise", "trial"
 * The returned string must be freed with talus_free_string().
 */
char* talus_license_tier(void);

/**
 * Checks if the current license allows a specific feature.
 *
 * @param feature   Feature name (e.g., "auto_kill", "web", "kafka").
 * @return 1 if allowed, 0 if not.
 */
int talus_license_allows(const char* feature);

/**
 * Returns 1 if a valid enterprise license is active.
 */
int talus_license_is_enterprise(void);

/**
 * Returns the license status as a JSON string.
 * The returned string must be freed with talus_free_string().
 */
char* talus_license_status(void);

/* ── Convenience macros ─────────────────────────────────────────────────── */

/** Check if a talus error code indicates success. */
#define TALUS_OK(err) ((err) == TALUS_OK)

/** Get the last error message (thread-local). */
const char* talus_last_error(void);

#ifdef __cplusplus
}
#endif

#endif /* TALUS_H */
