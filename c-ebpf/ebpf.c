// SPDX-License-Identifier: MIT
// ebpf.c — C library wrapping libbpf for Talus eBPF Process Monitor
//
// Handles eBPF object loading, tracepoint attachment, and perf buffer reading.
// Go calls this via CGo — all high-level monitoring logic lives in Go.

#include "talus.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>
#include <pthread.h>
#include <sys/time.h>

#include <bpf/libbpf.h>
#include <bpf/bpf.h>
#include <linux/perf_event.h>
#include <sys/syscall.h>

// ── Constants ────────────────────────────────────────────────────────────

#define PERF_BUF_PAGES    8
#define RAW_RING_SIZE     8192

// ── Thread-local error buffer ────────────────────────────────────────────

static __thread char tls_error[256];

const char *talus_last_error(void)
{
    return tls_error[0] ? tls_error : NULL;
}

static void set_error(const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(tls_error, sizeof(tls_error), fmt, ap);
    va_end(ap);
}

const char *talus_strerror(talus_err_t err)
{
    switch (err) {
    case TALUS_OK:            return "success";
    case TALUS_ERR_NOMEM:     return "out of memory";
    case TALUS_ERR_INVAL:     return "invalid argument";
    case TALUS_ERR_PERM:      return "permission denied";
    case TALUS_ERR_IO:        return "I/O error";
    case TALUS_ERR_NOT_FOUND: return "not found";
    default:                    return "unknown error";
    }
}

const char *talus_version(void)
{
    return "0.6.0";
}

// ── Monitor structure ────────────────────────────────────────────────────

struct talus_monitor {
    struct bpf_object *obj;
    struct perf_buffer *pb;

    // Lock-free raw ring buffer for perf callback → poll()
    struct {
        talus_raw_event_t ring[RAW_RING_SIZE];
        volatile int head; // written by perf callback
        volatile int tail; // read by poll()
    } raw_ring;

    uint64_t total_lost;
};

// ── Perf buffer callbacks ────────────────────────────────────────────────

static void perf_event_handler(void *ctx, int cpu, void *data, uint32_t size)
{
    (void)cpu;
    talus_monitor_t *m = (talus_monitor_t *)ctx;
    if (size < sizeof(talus_raw_event_t)) return;

    const talus_raw_event_t *raw = (const talus_raw_event_t *)data;

    // Lock-free single-producer ring buffer
    int h = m->raw_ring.head;
    int next = (h + 1) % RAW_RING_SIZE;
    if (next == m->raw_ring.tail) return; // full, drop event
    memcpy((void *)&m->raw_ring.ring[h], raw, sizeof(talus_raw_event_t));
    __sync_synchronize(); // memory barrier
    m->raw_ring.head = next;
}

static void perf_event_lost_handler(void *ctx, int cpu, __u64 cnt)
{
    (void)cpu;
    talus_monitor_t *m = (talus_monitor_t *)ctx;
    __sync_fetch_and_add(&m->total_lost, cnt);
}

// ── Monitor creation ─────────────────────────────────────────────────────

talus_err_t talus_monitor_create(const char *bpf_path,
                                     talus_monitor_t **out)
{
    if (!bpf_path || !out) {
        set_error("null pointer argument");
        return TALUS_ERR_INVAL;
    }

    if (access(bpf_path, R_OK) != 0) {
        set_error("eBPF object not found: %s", bpf_path);
        return TALUS_ERR_NOT_FOUND;
    }

    talus_monitor_t *m = calloc(1, sizeof(*m));
    if (!m) {
        set_error("out of memory");
        return TALUS_ERR_NOMEM;
    }

    m->raw_ring.head = 0;
    m->raw_ring.tail = 0;

    // Load eBPF object
    fprintf(stderr, "[talus] loading eBPF object: %s\n", bpf_path);
    m->obj = bpf_object__open(bpf_path);
    if (!m->obj) {
        set_error("failed to open eBPF object: %s", strerror(errno));
        free(m);
        return TALUS_ERR_IO;
    }

    if (bpf_object__load(m->obj) != 0) {
        set_error("failed to load eBPF object: %s", strerror(errno));
        bpf_object__close(m->obj);
        free(m);
        return TALUS_ERR_IO;
    }

    fprintf(stderr, "[talus] eBPF object loaded OK\n");

    // Attach tracepoints (required: execve + openat, optional: the rest)
    static const char *required_tps[] = {
        "sys_enter_execve", "sys_enter_openat", NULL
    };
    static const char *optional_tps[] = {
        "sys_enter_connect", "sys_enter_accept",
        "sys_enter_sendto", "sys_enter_recvfrom",
        "sys_enter_mkdir", "sys_enter_unlinkat",
        "sys_enter_kill", "sys_enter_fchmodat",
        NULL
    };

    for (int i = 0; required_tps[i]; i++) {
        struct bpf_program *prog = bpf_object__find_program_by_name(
            m->obj, required_tps[i]);
        if (!prog) {
            set_error("program not found: %s", required_tps[i]);
            bpf_object__close(m->obj);
            free(m);
            return TALUS_ERR_NOT_FOUND;
        }
        if (bpf_program__attach(prog) == NULL) {
            set_error("failed to attach: %s", required_tps[i]);
            bpf_object__close(m->obj);
            free(m);
            return TALUS_ERR_IO;
        }
        fprintf(stderr, "[talus] attached tracepoint syscalls/%s\n",
                required_tps[i]);
    }

    for (int i = 0; optional_tps[i]; i++) {
        struct bpf_program *prog = bpf_object__find_program_by_name(
            m->obj, optional_tps[i]);
        if (prog) {
            if (bpf_program__attach(prog)) {
                fprintf(stderr, "[talus] attached tracepoint syscalls/%s\n",
                        optional_tps[i]);
            } else {
                fprintf(stderr, "[talus] WARN: failed to attach %s\n",
                        optional_tps[i]);
            }
        } else {
            fprintf(stderr, "[talus] WARN: program %s not found\n",
                    optional_tps[i]);
        }
    }

    // Open perf buffer
    struct bpf_map *events_map = bpf_object__find_map_by_name(m->obj, "events");
    if (!events_map) {
        set_error("'events' map not found");
        bpf_object__close(m->obj);
        free(m);
        return TALUS_ERR_NOT_FOUND;
    }

    m->pb = perf_buffer__new(bpf_map__fd(events_map), PERF_BUF_PAGES,
                              perf_event_handler, perf_event_lost_handler,
                              m, NULL);
    if (!m->pb) {
        set_error("failed to create perf buffer: %s", strerror(errno));
        bpf_object__close(m->obj);
        free(m);
        return TALUS_ERR_IO;
    }

    fprintf(stderr, "[talus] perf buffer opened on %d CPUs\n",
            libbpf_num_possible_cpus());

    *out = m;
    return TALUS_OK;
}

// ── Monitor destruction ──────────────────────────────────────────────────

void talus_monitor_destroy(talus_monitor_t *m)
{
    if (!m) return;
    if (m->pb) perf_buffer__free(m->pb);
    if (m->obj) bpf_object__close(m->obj);
    free(m);
}

// ── Event polling ────────────────────────────────────────────────────────

talus_err_t talus_monitor_poll(talus_monitor_t *m,
                                   talus_raw_event_t *events,
                                   uint32_t max_events,
                                   uint32_t *count)
{
    if (!m || !events || !count) {
        set_error("null pointer argument");
        return TALUS_ERR_INVAL;
    }

    // Drain the perf buffer — this fills raw_ring via the callback
    if (m->pb) {
        perf_buffer__poll(m->pb, 0);
    }

    // Copy pending raw events from ring buffer to caller
    uint32_t n = 0;
    while (m->raw_ring.tail != m->raw_ring.head && n < max_events) {
        __sync_synchronize();
        memcpy(&events[n], &m->raw_ring.ring[m->raw_ring.tail],
               sizeof(talus_raw_event_t));
        m->raw_ring.tail = (m->raw_ring.tail + 1) % RAW_RING_SIZE;
        n++;
    }

    *count = n;
    return TALUS_OK;
}

// ── Lost events counter ──────────────────────────────────────────────────

uint64_t talus_monitor_get_lost(talus_monitor_t *m)
{
    if (!m) return 0;
    return m->total_lost;
}
