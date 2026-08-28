/**
 * halcyon.h — C API for Halcyon eBPF Process Monitor
 *
 * Provides eBPF loading, perf buffer management, and event polling.
 * Designed for Go CGo consumption — the Go layer handles all
 * high-level monitoring logic (sliding windows, alerts, stats, etc.).
 */

#ifndef HALCYON_H
#define HALCYON_H

#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── Version ────────────────────────────────────────────────────────────── */

#define HALCYON_VERSION_MAJOR 0
#define HALCYON_VERSION_MINOR 6
#define HALCYON_VERSION_PATCH 0

const char *halcyon_version(void);

/* ── Event type constants ───────────────────────────────────────────────── */

#define HALCYON_EVENT_EXECVE    0
#define HALCYON_EVENT_OPENAT    1
#define HALCYON_EVENT_CONNECT   2
#define HALCYON_EVENT_ACCEPT    3
#define HALCYON_EVENT_SENDTO    4
#define HALCYON_EVENT_RECVFROM  5
#define HALCYON_EVENT_MKDIR     6
#define HALCYON_EVENT_UNLINK    7
#define HALCYON_EVENT_KILL      8
#define HALCYON_EVENT_CHMOD     9

/* ── Buffer sizes (match eBPF kernel struct) ────────────────────────────── */

#define HALCYON_COMM_LEN       16
#define HALCYON_FILENAME_LEN   64
#define HALCYON_ARGV_LEN      128

/* ── Raw event record (matches eBPF kernel struct exactly) ─────────────── */

typedef struct {
    uint8_t  event_type;
    uint32_t pid;
    uint32_t uid;
    uint8_t  comm[HALCYON_COMM_LEN];
    uint8_t  filename[HALCYON_FILENAME_LEN];
    uint8_t  argv[HALCYON_ARGV_LEN];
} __attribute__((packed)) halcyon_raw_event_t;

/* ── Opaque handle ──────────────────────────────────────────────────────── */

typedef struct halcyon_monitor halcyon_monitor_t;

/* ── Error codes ────────────────────────────────────────────────────────── */

typedef enum {
    HALCYON_OK           =  0,
    HALCYON_ERR_NOMEM    = -1,
    HALCYON_ERR_INVAL    = -2,
    HALCYON_ERR_PERM     = -3,
    HALCYON_ERR_IO       = -4,
    HALCYON_ERR_NOT_FOUND = -5,
} halcyon_err_t;

const char *halcyon_strerror(halcyon_err_t err);
const char *halcyon_last_error(void);

/* ── Monitor lifecycle ──────────────────────────────────────────────────── */

halcyon_err_t halcyon_monitor_create(const char *bpf_path,
                                     halcyon_monitor_t **out);

void halcyon_monitor_destroy(halcyon_monitor_t *monitor);

/* ── Event polling ──────────────────────────────────────────────────────── */

/**
 * Polls the perf buffer and returns raw events.
 *
 * @param monitor    The monitor instance.
 * @param events     Caller-allocated array to receive raw events.
 * @param max_events Capacity of the events array.
 * @param count      Output: number of events written.
 * @return HALCYON_OK on success.
 */
halcyon_err_t halcyon_monitor_poll(halcyon_monitor_t *monitor,
                                   halcyon_raw_event_t *events,
                                   uint32_t max_events,
                                   uint32_t *count);

/**
 * Returns the total number of lost events (perf buffer overruns).
 */
uint64_t halcyon_monitor_get_lost(halcyon_monitor_t *monitor);

#ifdef __cplusplus
}
#endif

#endif /* HALCYON_H */
