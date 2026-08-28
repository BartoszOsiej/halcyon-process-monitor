/**
 * talus.h — C API for Talus eBPF Process Monitor
 *
 * Provides eBPF loading, perf buffer management, and event polling.
 * Designed for Go CGo consumption — the Go layer handles all
 * high-level monitoring logic (sliding windows, alerts, stats, etc.).
 */

#ifndef TALUS_H
#define TALUS_H

#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── Version ────────────────────────────────────────────────────────────── */

#define TALUS_VERSION_MAJOR 0
#define TALUS_VERSION_MINOR 6
#define TALUS_VERSION_PATCH 0

const char *talus_version(void);

/* ── Event type constants ───────────────────────────────────────────────── */

#define TALUS_EVENT_EXECVE    0
#define TALUS_EVENT_OPENAT    1
#define TALUS_EVENT_CONNECT   2
#define TALUS_EVENT_ACCEPT    3
#define TALUS_EVENT_SENDTO    4
#define TALUS_EVENT_RECVFROM  5
#define TALUS_EVENT_MKDIR     6
#define TALUS_EVENT_UNLINK    7
#define TALUS_EVENT_KILL      8
#define TALUS_EVENT_CHMOD     9

/* ── Buffer sizes (match eBPF kernel struct) ────────────────────────────── */

#define TALUS_COMM_LEN       16
#define TALUS_FILENAME_LEN   64
#define TALUS_ARGV_LEN      128

/* ── Raw event record (matches eBPF kernel struct exactly) ─────────────── */

typedef struct {
    uint8_t  event_type;
    uint32_t pid;
    uint32_t uid;
    uint8_t  comm[TALUS_COMM_LEN];
    uint8_t  filename[TALUS_FILENAME_LEN];
    uint8_t  argv[TALUS_ARGV_LEN];
} __attribute__((packed)) talus_raw_event_t;

/* ── Opaque handle ──────────────────────────────────────────────────────── */

typedef struct talus_monitor talus_monitor_t;

/* ── Error codes ────────────────────────────────────────────────────────── */

typedef enum {
    TALUS_OK           =  0,
    TALUS_ERR_NOMEM    = -1,
    TALUS_ERR_INVAL    = -2,
    TALUS_ERR_PERM     = -3,
    TALUS_ERR_IO       = -4,
    TALUS_ERR_NOT_FOUND = -5,
} talus_err_t;

const char *talus_strerror(talus_err_t err);
const char *talus_last_error(void);

/* ── Monitor lifecycle ──────────────────────────────────────────────────── */

talus_err_t talus_monitor_create(const char *bpf_path,
                                     talus_monitor_t **out);

void talus_monitor_destroy(talus_monitor_t *monitor);

/* ── Event polling ──────────────────────────────────────────────────────── */

/**
 * Polls the perf buffer and returns raw events.
 *
 * @param monitor    The monitor instance.
 * @param events     Caller-allocated array to receive raw events.
 * @param max_events Capacity of the events array.
 * @param count      Output: number of events written.
 * @return TALUS_OK on success.
 */
talus_err_t talus_monitor_poll(talus_monitor_t *monitor,
                                   talus_raw_event_t *events,
                                   uint32_t max_events,
                                   uint32_t *count);

/**
 * Returns the total number of lost events (perf buffer overruns).
 */
uint64_t talus_monitor_get_lost(talus_monitor_t *monitor);

#ifdef __cplusplus
}
#endif

#endif /* TALUS_H */
