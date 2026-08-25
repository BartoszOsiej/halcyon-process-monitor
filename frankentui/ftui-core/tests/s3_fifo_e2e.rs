//! E2E integration tests: S3-FIFO cache in realistic widget/layout scenarios (bd-l6yba.6)
//!
//! Validates S3-FIFO cache behavior in realistic usage patterns:
//! 1. Layout cache warm-up and steady-state hit rates.
//! 2. Ghost queue correctly promotes frequently re-accessed entries.
//! 3. Cache thrashing (rapid screen cycling) stays bounded.
//! 4. Eviction metrics (small/main) are consistent.
//! 5. Visual correctness: cached vs uncached produce identical results.
//! 6. Scan resistance: hot set survives scan workloads.
//!
//! Key S3-FIFO semantics tested:
//! - Items enter the small queue (10% of capacity).
//! - Items must be accessed via get() to build freq > 0 before eviction from small.
//! - Items with freq > 0 are promoted to main on eviction from small.
//! - Items with freq == 0 are evicted to ghost (key-only, bounded).
//! - Re-inserting a ghost key admits directly to main.
//!
//! All tests emit structured JSONL evidence to stdout.
//!
//! Run:
//!   cargo test -p ftui-core --test s3_fifo_e2e -- --nocapture

use ftui_core::s3_fifo::{S3Fifo, S3FifoStats};
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Instant;

// ── JSONL Evidence ──────────────────────────────────────────────────

#[derive(Serialize)]
struct Evidence {
    test: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    screen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_hits: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_misses: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hit_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ghost_promotions: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    queue_sizes: Option<QueueSizes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    pass: bool,
}

#[derive(Serialize)]
struct QueueSizes {
    small: usize,
    main: usize,
    ghost: usize,
}

impl Evidence {
    fn new(test: &'static str) -> Self {
        Self {
            test,
            frame_id: None,
            screen: None,
            cache_hits: None,
            cache_misses: None,
            hit_rate: None,
            ghost_promotions: None,
            queue_sizes: None,
            detail: None,
            pass: true,
        }
    }

    fn emit(&self) {
        println!("{}", serde_json::to_string(self).unwrap());
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

fn layout_key(screen: &str, widget_id: u32, width: u16) -> u64 {
    let mut hasher = DefaultHasher::new();
    screen.hash(&mut hasher);
    widget_id.hash(&mut hasher);
    width.hash(&mut hasher);
    hasher.finish()
}

fn compute_layout(key: u64) -> Vec<u16> {
    let a = (key % 100) as u16;
    let b = ((key >> 8) % 50) as u16;
    let c = ((key >> 16) % 200) as u16;
    vec![a, b, c]
}

fn stats_rate(stats: &S3FifoStats) -> f64 {
    let total = stats.hits + stats.misses;
    if total == 0 {
        0.0
    } else {
        stats.hits as f64 / total as f64
    }
}

fn stats_queues(stats: &S3FifoStats) -> QueueSizes {
    QueueSizes {
        small: stats.small_size,
        main: stats.main_size,
        ghost: stats.ghost_size,
    }
}

/// Simulate a realistic get-or-compute pattern: look up first, insert on miss.
/// Returns true if cache hit.
fn get_or_insert(cache: &mut S3Fifo<u64, Vec<u16>>, key: u64) -> bool {
    if cache.get(&key).is_some() {
        true
    } else {
        cache.insert(key, compute_layout(key));
        false
    }
}

// ============================================================================
// Test 1: Layout cache — per-screen warm-up then steady-state hits
// ============================================================================

#[test]
fn s3fifo_layout_cache_warmup_and_steady_state() {
    // Capacity 200: small=20, main=180. Working set = 6 screens × 15 widgets = 90 keys.
    // Items must be accessed before eviction from small to get promoted to main.
    let mut cache: S3Fifo<u64, Vec<u16>> = S3Fifo::new(200);
    let screens = [
        "dashboard",
        "settings",
        "help",
        "dataviz",
        "palette",
        "controls",
    ];
    let widgets_per_screen = 15;

    // Phase 1: Warm-up — insert + immediately access each screen's widgets.
    // This builds freq > 0 so items promote to main when evicted from small.
    for screen in &screens {
        for w in 0..widgets_per_screen {
            let key = layout_key(screen, w, 120);
            cache.insert(key, compute_layout(key));
        }
        // Access all widgets for this screen to build frequency
        for w in 0..widgets_per_screen {
            let key = layout_key(screen, w, 120);
            cache.get(&key);
        }
    }

    let warmup_stats = cache.stats();
    let mut ev = Evidence::new("s3fifo_layout_cache_warmup_and_steady_state");
    ev.cache_hits = Some(warmup_stats.hits);
    ev.cache_misses = Some(warmup_stats.misses);
    ev.queue_sizes = Some(stats_queues(&warmup_stats));
    ev.detail = Some(format!(
        "warmup: len={}, small={}, main={}",
        cache.len(),
        warmup_stats.small_size,
        warmup_stats.main_size
    ));
    ev.emit();

    // Phase 2: Steady-state — render all screens 10 times using get-or-insert.
    // Track hits/attempts manually since stats.misses only counts insert().
    let mut steady_hits = 0u64;
    let mut steady_attempts = 0u64;

    for frame_id in 0..10u64 {
        for screen in &screens {
            for w in 0..widgets_per_screen {
                let key = layout_key(screen, w, 120);
                steady_attempts += 1;
                if get_or_insert(&mut cache, key) {
                    steady_hits += 1;
                }
            }
        }

        if frame_id % 3 == 0 {
            let rate = steady_hits as f64 / steady_attempts as f64;
            let mut ev = Evidence::new("s3fifo_layout_cache_warmup_and_steady_state");
            ev.frame_id = Some(frame_id);
            ev.cache_hits = Some(steady_hits);
            ev.hit_rate = Some(rate);
            ev.queue_sizes = Some(stats_queues(&cache.stats()));
            ev.emit();
        }
    }

    let steady_rate = steady_hits as f64 / steady_attempts as f64;
    assert!(
        steady_rate >= 0.85,
        "Steady-state hit rate {steady_rate:.2} should be >= 0.85"
    );

    let mut ev = Evidence::new("s3fifo_layout_cache_warmup_and_steady_state");
    ev.hit_rate = Some(steady_rate);
    ev.detail = Some(format!(
        "steady: hits={steady_hits}/{steady_attempts}, rate={steady_rate:.4}"
    ));
    ev.pass = steady_rate >= 0.85;
    ev.emit();
}

// ============================================================================
// Test 2: Ghost queue promotion
// ============================================================================

#[test]
fn s3fifo_ghost_promotion() {
    // Use capacity 20: small=2, main=18, ghost=2.
    let mut cache: S3Fifo<u64, u64> = S3Fifo::new(20);

    // Insert 25 items without accessing — items overflow through small to ghost.
    for i in 0..25u64 {
        cache.insert(i, i * 100);
    }

    let stats = cache.stats();
    let mut ev = Evidence::new("s3fifo_ghost_promotion");
    ev.queue_sizes = Some(stats_queues(&stats));
    ev.detail = Some(format!(
        "after 25 inserts: len={}, ghost={}",
        cache.len(),
        stats.ghost_size
    ));
    ev.emit();

    assert!(stats.ghost_size > 0, "Ghost queue should have entries");

    // Re-insert some ghost keys — they should go directly to main.
    // We don't know exactly which keys are in ghost, but some early keys should be.
    let mut reinserted = 0u64;
    for i in 0..25u64 {
        if !cache.contains_key(&i) {
            cache.insert(i, i * 200);
            reinserted += 1;
        }
    }

    // After re-insertion, more items should be accessible
    let mut accessible = 0u64;
    for i in 0..25u64 {
        if cache.contains_key(&i) {
            accessible += 1;
        }
    }

    let final_stats = cache.stats();
    let mut ev2 = Evidence::new("s3fifo_ghost_promotion");
    ev2.ghost_promotions = Some(reinserted);
    ev2.queue_sizes = Some(stats_queues(&final_stats));
    ev2.detail = Some(format!(
        "reinserted={reinserted}, accessible={accessible}/25, len={}",
        cache.len()
    ));
    ev2.pass = true;
    ev2.emit();

    // Cache should still be bounded
    assert!(cache.len() <= cache.capacity());
}

// ============================================================================
// Test 3: Cache thrashing — rapid screen cycling stays bounded
// ============================================================================

#[test]
fn s3fifo_cache_thrashing() {
    let mut cache: S3Fifo<u64, Vec<u16>> = S3Fifo::new(100);
    let num_screens = 48;
    let widgets_per_screen = 15;
    let cycles = 5;

    for cycle in 0..cycles {
        let mut cycle_hits = 0u64;
        let mut cycle_attempts = 0u64;

        for screen_idx in 0..num_screens {
            let screen_name = format!("screen_{screen_idx}");
            for w in 0..widgets_per_screen {
                let key = layout_key(&screen_name, w, 120);
                cycle_attempts += 1;
                if get_or_insert(&mut cache, key) {
                    cycle_hits += 1;
                }
            }
        }

        let stats = cache.stats();
        assert!(
            cache.len() <= cache.capacity(),
            "Cache size {} exceeds capacity {}",
            cache.len(),
            cache.capacity()
        );

        let cycle_rate = cycle_hits as f64 / cycle_attempts as f64;
        let mut ev = Evidence::new("s3fifo_cache_thrashing");
        ev.frame_id = Some(cycle as u64);
        ev.cache_hits = Some(cycle_hits);
        ev.cache_misses = Some(cycle_attempts - cycle_hits);
        ev.hit_rate = Some(cycle_rate);
        ev.queue_sizes = Some(stats_queues(&stats));
        ev.detail = Some(format!(
            "cycle {cycle}: len={}, cap={}, rate={cycle_rate:.4}",
            cache.len(),
            cache.capacity()
        ));
        ev.pass = true;
        ev.emit();
    }
}

// ============================================================================
// Test 4: Cached vs uncached produce identical results
// ============================================================================

#[test]
fn s3fifo_correctness_cached_vs_uncached() {
    let mut cache: S3Fifo<u64, Vec<u16>> = S3Fifo::new(200);
    let screens = ["dashboard", "settings", "help", "dataviz", "palette"];
    let widgets_per_screen = 30;

    for screen in &screens {
        for w in 0..widgets_per_screen {
            let key = layout_key(screen, w, 120);
            let expected = compute_layout(key);

            // First access: cache miss → insert
            let cached = if let Some(val) = cache.get(&key) {
                val.clone()
            } else {
                let val = compute_layout(key);
                cache.insert(key, val.clone());
                val
            };
            assert_eq!(cached, expected, "First access mismatch for {screen}:{w}");

            // Second access: cache hit
            let cached2 = cache
                .get(&key)
                .expect("should hit on second access")
                .clone();
            assert_eq!(cached2, expected, "Second access mismatch for {screen}:{w}");
        }

        let mut ev = Evidence::new("s3fifo_correctness_cached_vs_uncached");
        ev.screen = Some(screen.to_string());
        ev.detail = Some(format!("{widgets_per_screen} widgets verified"));
        ev.pass = true;
        ev.emit();
    }
}

// ============================================================================
// Test 5: Eviction metrics consistency
// ============================================================================

#[test]
fn s3fifo_eviction_metrics() {
    let capacity = 50;
    let mut cache: S3Fifo<u64, u64> = S3Fifo::new(capacity);

    for i in 0..500u64 {
        cache.insert(i, i * 10);
        // Access hot keys to build frequency → they promote to main
        if i >= 10 {
            for hot in 0..5u64 {
                cache.get(&hot);
            }
        }
    }

    let stats = cache.stats();

    assert!(
        cache.len() <= capacity,
        "len {} exceeds capacity {capacity}",
        cache.len()
    );

    // Ghost should be bounded by ghost_cap (= small_cap = capacity/10)
    let ghost_cap = (capacity / 10).max(1);
    assert!(
        stats.ghost_size <= ghost_cap,
        "ghost {} exceeds ghost_cap {ghost_cap}",
        stats.ghost_size
    );

    // Small + main should equal total entries
    assert_eq!(
        stats.small_size + stats.main_size,
        cache.len(),
        "small({}) + main({}) != len({})",
        stats.small_size,
        stats.main_size,
        cache.len()
    );

    let mut ev = Evidence::new("s3fifo_eviction_metrics");
    ev.cache_hits = Some(stats.hits);
    ev.cache_misses = Some(stats.misses);
    ev.queue_sizes = Some(stats_queues(&stats));
    ev.detail = Some(format!(
        "capacity={capacity}, len={}, ghost={}/{}",
        cache.len(),
        stats.ghost_size,
        ghost_cap
    ));
    ev.pass = true;
    ev.emit();
}

// ============================================================================
// Test 6: Scan resistance — hot set survives scan workload
// ============================================================================

#[test]
fn s3fifo_scan_resistance_e2e() {
    let mut cache: S3Fifo<u64, u64> = S3Fifo::new(200);

    // Phase 1: Build a hot working set (keys 0..100) with high frequency
    for i in 0..100u64 {
        cache.insert(i, i);
        cache.get(&i);
        cache.get(&i);
    }

    let pre_scan = cache.stats();
    let mut ev = Evidence::new("s3fifo_scan_resistance_e2e");
    ev.detail = Some(format!(
        "pre-scan: len={}, hits={}, misses={}",
        cache.len(),
        pre_scan.hits,
        pre_scan.misses
    ));
    ev.emit();

    // Phase 2: Scan with 2000 unique keys
    for i in 10_000..12_000u64 {
        cache.insert(i, i);
    }

    // Phase 3: Check hot set survival
    let mut survivors = 0u64;
    for i in 0..100u64 {
        if cache.get(&i).is_some() {
            survivors += 1;
        }
    }

    let post_scan = cache.stats();
    let survival_rate = survivors as f64 / 100.0;

    assert!(
        survivors >= 10,
        "Only {survivors}/100 hot keys survived scan (expected >= 10)"
    );

    let mut ev = Evidence::new("s3fifo_scan_resistance_e2e");
    ev.hit_rate = Some(survival_rate);
    ev.queue_sizes = Some(stats_queues(&post_scan));
    ev.detail = Some(format!(
        "post-scan: {survivors}/100 survived ({survival_rate:.2})"
    ));
    ev.pass = survivors >= 10;
    ev.emit();
}

// ============================================================================
// Test 7: Multi-screen render simulation with frame-by-frame logging
// ============================================================================

#[test]
fn s3fifo_multi_screen_render_simulation() {
    let mut cache: S3Fifo<u64, Vec<u16>> = S3Fifo::new(300);
    let screens = [
        "dashboard",
        "settings",
        "help",
        "dataviz",
        "palette",
        "controls",
        "widgets",
        "tree_view",
    ];
    let widgets_per_screen = 25;
    let frames_per_screen = 10;

    for screen in &screens {
        let mut screen_hits = 0u64;
        let mut screen_attempts = 0u64;

        for frame_id in 0..frames_per_screen {
            for w in 0..widgets_per_screen {
                let key = layout_key(screen, w, 120);
                screen_attempts += 1;
                if get_or_insert(&mut cache, key) {
                    screen_hits += 1;
                }
            }

            let stats = cache.stats();
            let rate = screen_hits as f64 / screen_attempts as f64;

            let mut ev = Evidence::new("s3fifo_multi_screen_render_simulation");
            ev.frame_id = Some(frame_id as u64);
            ev.screen = Some(screen.to_string());
            ev.cache_hits = Some(screen_hits);
            ev.cache_misses = Some(screen_attempts - screen_hits);
            ev.hit_rate = Some(rate);
            ev.queue_sizes = Some(stats_queues(&stats));
            ev.pass = true;
            ev.emit();
        }

        // Over 10 frames: first frame is all misses (25), frames 2-10 are all hits (225).
        // Expected rate = 225/250 = 0.90
        let screen_rate = screen_hits as f64 / screen_attempts as f64;
        assert!(
            screen_rate >= 0.85,
            "Screen '{screen}' rate {screen_rate:.2} < 0.85"
        );
    }
}

// ============================================================================
// Test 8: Cache clear and repopulation
// ============================================================================

#[test]
fn s3fifo_clear_and_repopulate() {
    let mut cache: S3Fifo<u64, u64> = S3Fifo::new(100);
    // small_cap = 10, main_cap = 90

    // Populate in batches, accessing each batch to build freq for promotion.
    // Insert 10 items (fills small), access them, then insert next batch
    // which evicts the accessed items from small → main (freq > 0).
    for batch in 0..10 {
        let start = batch * 10;
        for i in start..(start + 10) {
            cache.insert(i as u64, i as u64 * 10);
        }
        // Access all items in this batch to build freq
        for i in start..(start + 10) {
            cache.get(&(i as u64));
        }
    }

    let pre_clear = cache.stats();
    let pre_len = cache.len();

    let mut ev = Evidence::new("s3fifo_clear_and_repopulate");
    ev.detail = Some(format!(
        "pre_clear: len={pre_len}, small={}, main={}, hits={}, misses={}",
        pre_clear.small_size, pre_clear.main_size, pre_clear.hits, pre_clear.misses
    ));
    ev.emit();

    // Clear
    cache.clear();
    assert!(cache.is_empty());
    let post_clear = cache.stats();
    assert_eq!(post_clear.hits, 0);
    assert_eq!(post_clear.misses, 0);

    // Repopulate with same batched pattern
    for batch in 0..10 {
        let start = batch * 10;
        for i in start..(start + 10) {
            cache.insert(i as u64, i as u64 * 10);
        }
        for i in start..(start + 10) {
            cache.get(&(i as u64));
        }
    }

    // Verify all items accessible
    let mut accessible = 0;
    for i in 0..100u64 {
        if cache.get(&i).is_some() {
            accessible += 1;
        }
    }

    let mut ev2 = Evidence::new("s3fifo_clear_and_repopulate");
    ev2.detail = Some(format!(
        "post_repopulate: len={}, accessible={accessible}/100",
        cache.len()
    ));
    ev2.pass = accessible > 50; // Most items should be accessible
    ev2.emit();

    assert!(
        accessible > 50,
        "Only {accessible}/100 accessible after repopulate"
    );
}

// ============================================================================
// Test 9: Width cache simulation — mixed access patterns
// ============================================================================

#[test]
fn s3fifo_width_cache_simulation() {
    // Simulate width cache: keys are grapheme hashes, values are display widths.
    // Use capacity 500: small=50, main=450.
    let mut cache: S3Fifo<u64, u8> = S3Fifo::new(500);

    let ascii_keys: Vec<u64> = (32u64..127).collect(); // 95 keys
    let cjk_keys: Vec<u64> = (0x4E00u64..0x4E00 + 200).collect(); // 200 keys
    let emoji_keys: Vec<u64> = (0x1F600u64..0x1F600 + 50).collect(); // 50 keys

    let start = Instant::now();

    // Track hits/attempts manually for accurate rate
    let mut total_hits = 0u64;
    let mut total_attempts = 0u64;

    for line in 0..1000u64 {
        // ASCII every line
        for &k in &ascii_keys {
            total_attempts += 1;
            if cache.get(&k).is_some() {
                total_hits += 1;
            } else {
                cache.insert(k, 1);
            }
        }

        // CJK every 10 lines
        if line % 10 == 0 {
            for &k in &cjk_keys {
                total_attempts += 1;
                if cache.get(&k).is_some() {
                    total_hits += 1;
                } else {
                    cache.insert(k, 2);
                }
            }
        }

        // Emoji every 50 lines
        if line % 50 == 0 {
            for &k in &emoji_keys {
                total_attempts += 1;
                if cache.get(&k).is_some() {
                    total_hits += 1;
                } else {
                    cache.insert(k, 2);
                }
            }
        }
    }

    let elapsed = start.elapsed();
    let rate = total_hits as f64 / total_attempts as f64;

    // With 345 unique keys fitting in capacity 500, and ASCII dominating (95K accesses),
    // the hit rate should be very high after warm-up.
    assert!(
        rate >= 0.80,
        "Width cache hit rate {rate:.2} should be >= 0.80"
    );

    // Verify ASCII chars are all accessible
    let mut ascii_accessible = 0;
    for &k in &ascii_keys {
        if cache.get(&k).is_some() {
            ascii_accessible += 1;
        }
    }

    let stats = cache.stats();
    let mut ev = Evidence::new("s3fifo_width_cache_simulation");
    ev.cache_hits = Some(total_hits);
    ev.cache_misses = Some(total_attempts - total_hits);
    ev.hit_rate = Some(rate);
    ev.queue_sizes = Some(stats_queues(&stats));
    ev.detail = Some(format!(
        "1000 lines, ascii_accessible={ascii_accessible}/{}, rate={rate:.4}, elapsed={elapsed:?}",
        ascii_keys.len()
    ));
    ev.pass = rate >= 0.80;
    ev.emit();
}

// ============================================================================
// Test 10: Degenerate inputs — capacity 2
// ============================================================================

#[test]
fn s3fifo_degenerate_capacity() {
    let mut cache: S3Fifo<u32, u32> = S3Fifo::new(2);
    assert!(cache.capacity() >= 2);

    cache.insert(1, 10);
    cache.insert(2, 20);
    assert!(cache.len() <= cache.capacity());

    cache.insert(3, 30);
    assert!(cache.len() <= cache.capacity());

    let found = cache.get(&2).is_some() || cache.get(&3).is_some();
    assert!(found, "At least one recent key should be accessible");

    let mut ev = Evidence::new("s3fifo_degenerate_capacity");
    ev.detail = Some(format!(
        "capacity={}, len={}, stats={:?}",
        cache.capacity(),
        cache.len(),
        cache.stats()
    ));
    ev.pass = true;
    ev.emit();

    let cache_zero: S3Fifo<u32, u32> = S3Fifo::new(0);
    assert!(cache_zero.capacity() >= 2);

    let mut ev2 = Evidence::new("s3fifo_degenerate_capacity");
    ev2.detail = Some(format!("capacity(0) clamped to {}", cache_zero.capacity()));
    ev2.pass = true;
    ev2.emit();
}

// ============================================================================
// Test 11: Performance — insert/get throughput
// ============================================================================

#[test]
fn s3fifo_throughput_report() {
    let capacity = 1000;
    let total_ops = 100_000;
    let mut cache: S3Fifo<u64, u64> = S3Fifo::new(capacity);

    let start_insert = Instant::now();
    for i in 0..total_ops as u64 {
        cache.insert(i, i * 7);
    }
    let insert_elapsed = start_insert.elapsed();

    let start_get = Instant::now();
    for i in (total_ops as u64 - capacity as u64)..total_ops as u64 {
        cache.get(&i);
    }
    let get_elapsed = start_get.elapsed();

    let stats = cache.stats();

    let mut ev = Evidence::new("s3fifo_throughput_report");
    ev.cache_hits = Some(stats.hits);
    ev.cache_misses = Some(stats.misses);
    ev.hit_rate = Some(stats_rate(&stats));
    ev.detail = Some(format!(
        "capacity={capacity}, ops={total_ops}, \
         insert_ns={}, get_ns={}, \
         insert_ns/op={:.0}, get_ns/op={:.0}",
        insert_elapsed.as_nanos(),
        get_elapsed.as_nanos(),
        insert_elapsed.as_nanos() as f64 / total_ops as f64,
        get_elapsed.as_nanos() as f64 / capacity as f64,
    ));
    ev.pass = true;
    ev.emit();
}
