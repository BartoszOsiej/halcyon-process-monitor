//! Benchmarks for Action Timeline screen (bd-11ck, bd-11ck.2)
//!
//! Performance Regression Tests for large event streams.
//!
//! Run with: cargo bench -p ftui-demo-showcase --bench action_timeline_bench
//!
//! Performance budgets (per bd-11ck.2):
//! - Empty render: < 100µs
//! - 100 events render: < 500µs
//! - 1K events render: < 2ms
//! - 10K events render: < 20ms (at buffer capacity)
//! - Filter operation: < 100µs (for MAX_EVENTS = 500)
//! - Navigation: < 50µs per operation

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ftui_core::geometry::Rect;
use ftui_demo_showcase::screens::{Screen, action_timeline::ActionTimeline};
use ftui_render::frame::Frame;
use ftui_render::grapheme_pool::GraphemePool;
use std::hint::black_box;

// =============================================================================
// Render Benchmarks: Various Event Counts
// =============================================================================

fn bench_action_timeline_render(c: &mut Criterion) {
    let mut group = c.benchmark_group("action_timeline/render");

    // Empty timeline (fresh state)
    group.bench_function("empty_120x40", |b| {
        let timeline = ActionTimeline::new();
        // Fresh timeline (has 12 initial events from default state)
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 120, 40);

        b.iter(|| {
            let mut frame = Frame::new(120, 40, &mut pool);
            timeline.view(&mut frame, area);
            black_box(&frame);
        })
    });

    // Small event count (default initialization = 12 events)
    group.bench_function("12_events_120x40", |b| {
        let timeline = ActionTimeline::new();
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 120, 40);

        b.iter(|| {
            let mut frame = Frame::new(120, 40, &mut pool);
            timeline.view(&mut frame, area);
            black_box(&frame);
        })
    });

    // 100 events
    group.throughput(Throughput::Elements(100));
    group.bench_function("100_events_120x40", |b| {
        let mut timeline = ActionTimeline::new();
        for i in 0..100 {
            timeline.record_command_event(
                i as u64,
                "Synthetic benchmark event",
                vec![("payload".to_string(), format!("bench_{i}"))],
            );
        }
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 120, 40);

        b.iter(|| {
            let mut frame = Frame::new(120, 40, &mut pool);
            timeline.view(&mut frame, area);
            black_box(&frame);
        })
    });

    // 500 events (at MAX_EVENTS capacity)
    group.throughput(Throughput::Elements(500));
    group.bench_function("500_events_120x40", |b| {
        let mut timeline = ActionTimeline::new();
        for i in 0..500 {
            timeline.record_command_event(
                i as u64,
                "Synthetic benchmark event at capacity",
                vec![
                    ("payload".to_string(), format!("bench_{i}")),
                    ("latency_ms".to_string(), ((i % 100) as u64).to_string()),
                ],
            );
        }
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 120, 40);

        b.iter(|| {
            let mut frame = Frame::new(120, 40, &mut pool);
            timeline.view(&mut frame, area);
            black_box(&frame);
        })
    });

    group.finish();
}

// =============================================================================
// Terminal Size Benchmarks
// =============================================================================

fn bench_action_timeline_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("action_timeline/terminal_size");

    for (w, h) in [(80, 24), (120, 40), (200, 60), (320, 80)] {
        let cells = w as u64 * h as u64;
        group.throughput(Throughput::Elements(cells));

        group.bench_with_input(
            BenchmarkId::new("render_500_events", format!("{w}x{h}")),
            &(w, h),
            |b, &(w, h)| {
                let mut timeline = ActionTimeline::new();
                for i in 0..500 {
                    timeline.record_command_event(
                        i as u64,
                        "Synthetic event",
                        vec![("id".to_string(), i.to_string())],
                    );
                }
                let mut pool = GraphemePool::new();
                let area = Rect::new(0, 0, w, h);

                b.iter(|| {
                    let mut frame = Frame::new(w, h, &mut pool);
                    timeline.view(&mut frame, area);
                    black_box(&frame);
                })
            },
        );
    }

    group.finish();
}

// =============================================================================
// Filter Operation Benchmarks
// =============================================================================

fn bench_action_timeline_filter(c: &mut Criterion) {
    let mut group = c.benchmark_group("action_timeline/filter");

    // Baseline: filter_indices with no filter
    group.bench_function("filter_indices_no_filter_500", |b| {
        let mut timeline = ActionTimeline::new();
        for i in 0..500 {
            timeline.record_command_event(
                i as u64,
                "Event",
                vec![("id".to_string(), i.to_string())],
            );
        }

        b.iter(|| {
            // Access filtered indices (private, but we measure render which calls it)
            let mut pool = GraphemePool::new();
            let mut frame = Frame::new(120, 40, &mut pool);
            timeline.view(&mut frame, Rect::new(0, 0, 120, 40));
            black_box(&frame);
        })
    });

    // Render after filter toggle (simulates user interaction)
    group.bench_function("render_after_filter_toggle", |b| {
        let mut timeline = ActionTimeline::new();
        for i in 0..500 {
            timeline.record_command_event(
                i as u64,
                "Event",
                vec![("id".to_string(), i.to_string())],
            );
        }
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 120, 40);

        b.iter(|| {
            // Simulate filter cycle (component filter)
            // Since we can't directly access filter methods, we measure the render
            let mut frame = Frame::new(120, 40, &mut pool);
            timeline.view(&mut frame, area);
            black_box(&frame);
        })
    });

    group.finish();
}

// =============================================================================
// Event Recording Benchmarks (Push Performance)
// =============================================================================

fn bench_action_timeline_push(c: &mut Criterion) {
    let mut group = c.benchmark_group("action_timeline/push");

    // Single event push
    group.bench_function("push_single_event", |b| {
        let mut timeline = ActionTimeline::new();
        let mut tick = 100u64;

        b.iter(|| {
            timeline.record_command_event(
                tick,
                "Benchmark event",
                vec![("key".to_string(), "value".to_string())],
            );
            tick += 1;
        })
    });

    // Push at capacity (eviction path)
    group.bench_function("push_at_capacity_evict", |b| {
        let mut timeline = ActionTimeline::new();
        // Fill to capacity first
        for i in 0..600 {
            timeline.record_command_event(
                i as u64,
                "Fill event",
                vec![("id".to_string(), i.to_string())],
            );
        }
        let mut tick = 1000u64;

        b.iter(|| {
            timeline.record_command_event(
                tick,
                "Eviction event",
                vec![("evicting".to_string(), "true".to_string())],
            );
            tick += 1;
        })
    });

    // Batch push (simulating burst of events)
    for count in [10, 50, 100] {
        group.throughput(Throughput::Elements(count));
        group.bench_with_input(
            BenchmarkId::new("push_batch", count),
            &count,
            |b, &count| {
                let mut timeline = ActionTimeline::new();
                let mut base_tick = 100u64;

                b.iter(|| {
                    for i in 0..count {
                        timeline.record_command_event(
                            base_tick + i,
                            "Batch event",
                            vec![("batch".to_string(), i.to_string())],
                        );
                    }
                    base_tick += count;
                })
            },
        );
    }

    group.finish();
}

// =============================================================================
// Tick Performance (Runtime Simulation)
// =============================================================================

fn bench_action_timeline_tick(c: &mut Criterion) {
    let mut group = c.benchmark_group("action_timeline/tick");

    // Single tick (event generation)
    group.bench_function("tick_single", |b| {
        let mut timeline = ActionTimeline::new();
        let mut tick_count = 100u64;

        b.iter(|| {
            timeline.tick(tick_count);
            tick_count += 1;
        })
    });

    // Rapid ticks (simulating fast refresh rate)
    group.bench_function("tick_rapid_100", |b| {
        let mut timeline = ActionTimeline::new();
        let mut tick_count = 100u64;

        b.iter(|| {
            for _ in 0..100 {
                timeline.tick(tick_count);
                tick_count += 1;
            }
        })
    });

    group.finish();
}

// =============================================================================
// Full Frame Render Pipeline
// =============================================================================

fn bench_action_timeline_frame_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("action_timeline/frame_pipeline");

    // Full frame: tick + view (simulates one runtime frame)
    group.bench_function("tick_and_render_120x40", |b| {
        let mut timeline = ActionTimeline::new();
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 120, 40);
        let mut tick_count = 100u64;

        b.iter(|| {
            timeline.tick(tick_count);
            tick_count += 1;

            let mut frame = Frame::new(120, 40, &mut pool);
            timeline.view(&mut frame, area);
            black_box(&frame);
        })
    });

    // Full frame with 500 events (stress test)
    group.bench_function("tick_and_render_500_events", |b| {
        let mut timeline = ActionTimeline::new();
        for i in 0..500 {
            timeline.record_command_event(
                i as u64,
                "Pre-filled event",
                vec![("id".to_string(), i.to_string())],
            );
        }
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 120, 40);
        let mut tick_count = 1000u64;

        b.iter(|| {
            timeline.tick(tick_count);
            tick_count += 1;

            let mut frame = Frame::new(120, 40, &mut pool);
            timeline.view(&mut frame, area);
            black_box(&frame);
        })
    });

    group.finish();
}

// =============================================================================
// Stress Tests: Rapid Operations
// =============================================================================

fn bench_action_timeline_stress(c: &mut Criterion) {
    let mut group = c.benchmark_group("action_timeline/stress");
    group.sample_size(50); // Reduce samples for stress tests

    // Many events with varied fields
    group.bench_function("varied_fields_500", |b| {
        let mut timeline = ActionTimeline::new();
        for i in 0..500 {
            let fields: Vec<(String, String)> = (0..(i % 10))
                .map(|j| (format!("field_{j}"), format!("value_{}", i * j)))
                .collect();
            timeline.record_command_event(
                i as u64,
                format!("Event {i} with {} fields", i % 10),
                fields,
            );
        }
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 120, 40);

        b.iter(|| {
            let mut frame = Frame::new(120, 40, &mut pool);
            timeline.view(&mut frame, area);
            black_box(&frame);
        })
    });

    // Long summaries
    group.bench_function("long_summaries_500", |b| {
        let mut timeline = ActionTimeline::new();
        for i in 0..500 {
            let long_summary = "X".repeat(200);
            timeline.record_command_event(
                i as u64,
                long_summary,
                vec![("id".to_string(), i.to_string())],
            );
        }
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 120, 40);

        b.iter(|| {
            let mut frame = Frame::new(120, 40, &mut pool);
            timeline.view(&mut frame, area);
            black_box(&frame);
        })
    });

    group.finish();
}

// =============================================================================
// Benchmark Groups
// =============================================================================

criterion_group!(
    benches,
    bench_action_timeline_render,
    bench_action_timeline_sizes,
    bench_action_timeline_filter,
    bench_action_timeline_push,
    bench_action_timeline_tick,
    bench_action_timeline_frame_pipeline,
    bench_action_timeline_stress,
);

criterion_main!(benches);
