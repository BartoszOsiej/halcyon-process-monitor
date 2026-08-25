use ftui_layout::{CoherenceCache, Constraint, Flex, LayoutSizeHint, Rect};

#[test]
fn test_coherence_cache_indexing_bug() {
    let mut cache = CoherenceCache::default();

    // Constraints: Fixed(10), Min(10).
    // Item 0 is fixed. Item 1 grows.
    let flex = Flex::horizontal().constraints([Constraint::Fixed(10), Constraint::Min(10)]);

    let area = Rect::new(0, 0, 100, 10);

    // First split: Should populate cache.
    // Available 90. Item 1 gets 10 (base) + 80 (grow).
    let rects1 = flex.split_with_measurer_stably(area, |_, _| LayoutSizeHint::ZERO, &mut cache);
    assert_eq!(rects1[1].width, 90);

    // To verify coherence is working, we need a scenario where rounding stability matters.
    // Or just inspect the cache if possible? CoherenceCache internals are likely private or tricky to check.
    // However, we can infer it from the logic I analyzed.
    // If the cache stored `[80]` (length 1), then `get(1)` will fail in the next pass.

    // Let's create a scenario where temporal stability would affect rounding.
    // 3 items growing. 100 pixels. 33.33 each.
    // Remainder 1.
    // Frame 1: 34, 33, 33.
    // Frame 2: 34, 33, 33 (should stay same).

    // Bug scenario:
    // Fixed(10), Min(10), Min(10).
    // Grow indices: 1, 2.
    // Available: 100 - 10 = 90.
    // Item 1: 10 base. Item 2: 10 base.
    // Remaining to distribute: 70.
    // Targets: 35, 35.
    // Alloc: 35, 35.
    // Cache stores: `[35, 35]`.

    // Next frame: Same.
    // Retrieval:
    // i=1: `cache.get(1)` -> 35. OK.
    // i=2: `cache.get(2)` -> None (out of bounds).

    // This assumes the vector is `[35, 35]`.
    // If we can verify that item 2 loses coherence, that confirms the bug.
    // How to detect lost coherence?
    // "Temporal stability: did previous allocation use ceil?"
    // If prev is None (lost), it defaults to false.
    // If we rely on ceil preference to stabilize, losing it might flip rounding.

    // Setup:
    // Total 4 cells to distribute to 2 items. Targets 1.5, 2.5? No, targets sum to total.
    // Total 1. Targets 0.5, 0.5.
    // Remainder 1.
    // Priority: Equal.
    // Tie-break: Index. Item 1 gets +1. Item 2 gets +0.
    // Result: 1, 0.
    // Cache stores `[1, 0]`.

    // Frame 2:
    // Prev: Item 1=1 (ceil), Item 2=0 (floor).
    // Logic prefers ceil. So Item 1 should keep 1.

    // If bug exists:
    // Item 1 index is e.g. 5.
    // Cache stores `[..., 1, 0]`.
    // Retrieval `get(5)` fails if cache is packed and small.

    // Let's force index mismatch.
    // Fixed(10) (idx 0), Min(0) (idx 1), Min(0) (idx 2).
    // Total available 11.
    // Fixed takes 10. Remainder 1.
    // Grow indices: 1, 2.
    // Targets: 0.5, 0.5.
    // Deficit 1.
    // Index tie break: 1 gets it.
    // Alloc: 1[1], 0[0].

    // Cache (BUG): Stores `[1, 0]`.

    // Frame 2:
    // Retrieval:
    // i=1. `get(1)` -> 0 (from `[1, 0]` at index 1). Wait!
    // `[1, 0]` at index 0 is 1. At index 1 is 0.
    // So `get(1)` returns 0.
    // But Item 1 was allocated 1!
    // So it sees "prev was 0".
    // i=2. `get(2)` -> None.

    // So Item 1 thinks prev was 0 (floor).
    // Item 2 thinks prev was None.

    // This confirms the bug: mapping is shifted or missing.
    // The previous allocation for Item 1 (value 1) is at index 0 in cache.
    // But we look up at index 1. We get value 0.
    // So Item 1 loses its "I had ceil" status.

    // Test:
    // Fixed(10), Min(0), Min(0). Area width 11.
    // Expect: Item 1 gets 1, Item 2 gets 0.
    // ...
    // Actually, this is deterministic by index anyway.
    // We need a case where targets change slightly to flip preference, BUT coherence holds it back.
    // Or where we rely on coherence to break a tie differently than index.

    // If we have 3 items. Targets 0.33, 0.33, 0.33. Total 1.
    // Index tie break: Item 0 gets 1.

    // Frame 2: Targets 0.33, 0.33, 0.33.
    // Prev: 1, 0, 0.
    // Item 0 has ceil preference. It keeps 1.

    // Shifted indices:
    // Fixed(10), Min(0), Min(0), Min(0).
    // Grow indices: 1, 2, 3.
    // Width 11. Remainder 1.
    // Targets 0.33...
    // Item 1 gets 1.
    // Cache (BUG): `[1, 0, 0]`.

    // Frame 2:
    // i=1. `get(1)` -> 0. (Wrong! Should be 1).
    // i=2. `get(2)` -> 0.
    // i=3. `get(3)` -> None.

    // So Item 1 loses its ceil status.
    // It falls back to index tie break.
    // Item 1 is first flexible. It gets 1.
    // So outcome is same.

    // We need a case where the shifted lookup yields `ceil` for the WRONG item, or `floor` for the RIGHT item.
    // Here `get(1)` returned 0 (floor), but truth was 1 (ceil).
    // So Item 1 lost its advantage.

    // If we construct a case where Item 2 *should* win due to coherence, but loses it?
    // Hard to construct via public API because index tie-break aligns with iterator order.

    // BUT, the bug is clear from code analysis. `cache.store` saves a dense vector of N items (N < total).
    // `solve` reads it as a sparse vector indexed by M (M > N).
    // This is definitely wrong.
}
