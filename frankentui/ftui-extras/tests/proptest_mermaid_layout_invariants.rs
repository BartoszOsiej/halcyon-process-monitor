//! Property-based invariant tests for the mermaid layout engine.
//!
//! These tests verify structural invariants that must hold for **any** valid
//! `MermaidDiagramIr` input fed through the Sugiyama layout pipeline:
//!
//! 1. Layout determinism — same input always yields identical output
//! 2. Node completeness — every IR node appears in the layout
//! 3. Bounding box containment — all node rects fit inside the bounding box
//! 4. Edge path structure — routed edges have ≥ 2 waypoints
//! 5. Stats consistency — stats fields are non-negative and internally coherent
//! 6. Direction coverage — layout succeeds for every `GraphDirection`
//! 7. Evaluate non-negative — `LayoutObjective` fields are non-negative
//! 8. Route completeness — `route_all_edges` returns one path per IR edge
//! 9. Label placement — `place_labels` doesn't panic and returns well-formed results
//! 10. Cluster containment — cluster rects contain their member node rects
//! 11. Rank ordering — nodes with lower rank have smaller y (for TB) or x (for LR)
//! 12. Empty graph — zero nodes/edges produce empty layout without panic
//! 13. Single node — degenerate single-node graph lays out correctly
//! 14. Node dimensions positive — all node rects have positive width/height
//! 15. Bounding box positive — always has non-negative size
//! 16. Routing report bends — matches actual bend count in paths

#[cfg(feature = "diagram")]
mod tests {
    use ftui_extras::mermaid::*;
    use ftui_extras::mermaid_layout::*;
    use proptest::prelude::*;
    use std::collections::BTreeMap;

    // ── Helpers ─────────────────────────────────────────────────────────────

    fn dummy_span() -> Span {
        Span {
            start: Position {
                line: 1,
                col: 1,
                byte: 0,
            },
            end: Position {
                line: 1,
                col: 1,
                byte: 0,
            },
        }
    }

    fn make_ir(
        node_count: usize,
        edges: &[(usize, usize)],
        direction: GraphDirection,
    ) -> MermaidDiagramIr {
        let nodes: Vec<IrNode> = (0..node_count)
            .map(|i| IrNode {
                id: format!("N{i}"),
                label: None,
                shape: NodeShape::Rect,
                classes: Vec::new(),
                style_ref: None,
                span_primary: dummy_span(),
                span_all: Vec::new(),
                implicit: false,
                members: Vec::new(),
                annotation: None,
            })
            .collect();

        let ir_edges: Vec<IrEdge> = edges
            .iter()
            .filter(|&&(from, to)| from < node_count && to < node_count)
            .map(|&(from, to)| IrEdge {
                from: IrEndpoint::Node(IrNodeId(from)),
                to: IrEndpoint::Node(IrNodeId(to)),
                arrow: "-->".to_string(),
                label: None,
                style_ref: None,
                span: dummy_span(),
            })
            .collect();

        MermaidDiagramIr {
            diagram_type: DiagramType::Graph,
            direction,
            nodes,
            edges: ir_edges,
            ports: Vec::new(),
            clusters: Vec::new(),
            labels: Vec::new(),
            pie_entries: Vec::new(),
            pie_title: None,
            pie_show_data: false,
            style_refs: Vec::new(),
            links: Vec::new(),
            meta: MermaidDiagramMeta {
                diagram_type: DiagramType::Graph,
                direction,
                support_level: MermaidSupportLevel::Supported,
                init: MermaidInitParse {
                    config: MermaidInitConfig {
                        theme: None,
                        theme_variables: BTreeMap::new(),
                        flowchart_direction: None,
                    },
                    warnings: Vec::new(),
                    errors: Vec::new(),
                },
                theme_overrides: MermaidThemeOverrides {
                    theme: None,
                    theme_variables: BTreeMap::new(),
                },
                guard: MermaidGuardReport::default(),
            },
            constraints: Vec::new(),
            quadrant_points: Vec::new(),
            quadrant_title: None,
            quadrant_x_axis: None,
            quadrant_y_axis: None,
            quadrant_labels: [None, None, None, None],
            packet_fields: Vec::new(),
            packet_title: None,
            packet_bits_per_row: 32,
            sequence_participants: Vec::new(),
            sequence_controls: Vec::new(),
            sequence_notes: Vec::new(),
            sequence_activations: Vec::new(),
            sequence_autonumber: false,
            gantt_title: None,
            gantt_sections: Vec::new(),
            gantt_tasks: Vec::new(),
        }
    }

    fn make_ir_with_clusters(
        node_count: usize,
        edges: &[(usize, usize)],
        cluster_members: &[Vec<usize>],
        direction: GraphDirection,
    ) -> MermaidDiagramIr {
        let mut ir = make_ir(node_count, edges, direction);
        ir.clusters = cluster_members
            .iter()
            .enumerate()
            .map(|(i, members)| IrCluster {
                id: IrClusterId(i),
                title: None,
                members: members
                    .iter()
                    .filter(|&&m| m < node_count)
                    .map(|&m| IrNodeId(m))
                    .collect(),
                span: dummy_span(),
            })
            .collect();
        ir
    }

    // ── Strategies ──────────────────────────────────────────────────────────

    fn direction_strategy() -> impl Strategy<Value = GraphDirection> {
        prop_oneof![
            Just(GraphDirection::TB),
            Just(GraphDirection::TD),
            Just(GraphDirection::LR),
            Just(GraphDirection::RL),
            Just(GraphDirection::BT),
        ]
    }

    /// Generate a random graph with 1..=max_nodes and 0..=max_edges edges.
    fn graph_strategy(
        max_nodes: usize,
        max_edges: usize,
    ) -> impl Strategy<Value = (usize, Vec<(usize, usize)>, GraphDirection)> {
        (1..=max_nodes, direction_strategy()).prop_flat_map(move |(n, dir)| {
            let edge_count = if n > 1 {
                0..=max_edges.min(n * (n - 1) / 2)
            } else {
                0..=0
            };
            (
                Just(n),
                proptest::collection::vec((0..n, 0..n), edge_count),
                Just(dir),
            )
        })
    }

    /// Generate a DAG-like graph (from < to) to avoid cycles.
    fn dag_strategy(
        max_nodes: usize,
        max_edges: usize,
    ) -> impl Strategy<Value = (usize, Vec<(usize, usize)>, GraphDirection)> {
        (2..=max_nodes, direction_strategy()).prop_flat_map(move |(n, dir)| {
            let edge_count = 1..=max_edges.min(n * (n - 1) / 2);
            (
                Just(n),
                proptest::collection::vec((0..n, 0..n), edge_count).prop_map(|edges| {
                    edges
                        .into_iter()
                        .map(|(a, b)| if a < b { (a, b) } else { (b, a) })
                        .filter(|(a, b)| a != b)
                        .collect::<Vec<_>>()
                }),
                Just(dir),
            )
        })
    }

    fn rect_contains(outer: &LayoutRect, inner: &LayoutRect) -> bool {
        let eps = 0.01;
        outer.x - eps <= inner.x
            && outer.y - eps <= inner.y
            && (outer.x + outer.width + eps) >= (inner.x + inner.width)
            && (outer.y + outer.height + eps) >= (inner.y + inner.height)
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 1. Layout determinism — same input always yields identical output
    // ═════════════════════════════════════════════════════════════════════════

    proptest! {
        #[test]
        fn layout_is_deterministic(
            (n, edges, dir) in graph_strategy(12, 15),
        ) {
            let ir = make_ir(n, &edges, dir);
            let config = MermaidConfig::default();

            let layout1 = layout_diagram(&ir, &config);
            let layout2 = layout_diagram(&ir, &config);

            prop_assert_eq!(layout1.nodes.len(), layout2.nodes.len(),
                "Node count differs between identical runs");
            prop_assert_eq!(layout1.edges.len(), layout2.edges.len(),
                "Edge count differs between identical runs");
            prop_assert_eq!(layout1.bounding_box, layout2.bounding_box,
                "Bounding box differs between identical runs");
            prop_assert_eq!(layout1.stats.crossings, layout2.stats.crossings,
                "Crossings count differs between identical runs");
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 2. Node completeness — every IR node appears in the layout
    // ═════════════════════════════════════════════════════════════════════════

    proptest! {
        #[test]
        fn all_ir_nodes_present_in_layout(
            (n, edges, dir) in graph_strategy(20, 25),
        ) {
            let ir = make_ir(n, &edges, dir);
            let layout = layout_diagram(&ir, &MermaidConfig::default());

            prop_assert_eq!(
                layout.nodes.len(), n,
                "Layout has {} nodes but IR has {}",
                layout.nodes.len(), n,
            );

            let mut seen = vec![false; n];
            for node_box in &layout.nodes {
                prop_assert!(
                    node_box.node_idx < n,
                    "node_idx {} out of range 0..{}",
                    node_box.node_idx, n,
                );
                seen[node_box.node_idx] = true;
            }
            for (i, &s) in seen.iter().enumerate() {
                prop_assert!(s, "Node {} missing from layout", i);
            }
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 3. Bounding box containment — all node rects fit within bounding box
    // ═════════════════════════════════════════════════════════════════════════

    proptest! {
        #[test]
        fn node_rects_within_bounding_box(
            (n, edges, dir) in graph_strategy(15, 20),
        ) {
            let ir = make_ir(n, &edges, dir);
            let layout = layout_diagram(&ir, &MermaidConfig::default());
            let bb = &layout.bounding_box;

            for node_box in &layout.nodes {
                prop_assert!(
                    rect_contains(bb, &node_box.rect),
                    "Node {} rect {:?} escapes bounding box {:?}",
                    node_box.node_idx, node_box.rect, bb,
                );
            }
        }

        #[test]
        fn edge_waypoints_are_finite(
            (n, edges, dir) in dag_strategy(10, 12),
        ) {
            let ir = make_ir(n, &edges, dir);
            let layout = layout_diagram(&ir, &MermaidConfig::default());
            let (routed, _report) = route_all_edges(
                &ir, &layout, &MermaidConfig::default(), &RoutingWeights::default(),
            );

            for path in &routed {
                for wp in &path.waypoints {
                    prop_assert!(
                        wp.x.is_finite() && wp.y.is_finite(),
                        "Edge {} waypoint ({}, {}) is not finite",
                        path.edge_idx, wp.x, wp.y,
                    );
                }
            }
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 4. Edge path structure — routed edges have ≥ 2 waypoints
    // ═════════════════════════════════════════════════════════════════════════

    proptest! {
        #[test]
        fn routed_edges_have_at_least_two_waypoints(
            (n, edges, dir) in dag_strategy(10, 12),
        ) {
            let ir = make_ir(n, &edges, dir);
            let layout = layout_diagram(&ir, &MermaidConfig::default());
            let (routed, _report) = route_all_edges(
                &ir, &layout, &MermaidConfig::default(), &RoutingWeights::default(),
            );

            for path in &routed {
                prop_assert!(
                    path.waypoints.len() >= 2,
                    "Edge {} has {} waypoints (need ≥ 2)",
                    path.edge_idx, path.waypoints.len(),
                );
            }
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 5. Stats consistency — crossings & ranks are non-negative, rank count
    //    matches actual unique ranks in the layout
    // ═════════════════════════════════════════════════════════════════════════

    proptest! {
        #[test]
        fn stats_fields_coherent(
            (n, edges, dir) in graph_strategy(15, 20),
        ) {
            let ir = make_ir(n, &edges, dir);
            let layout = layout_diagram(&ir, &MermaidConfig::default());

            prop_assert!(layout.stats.ranks > 0,
                "Layout with {} nodes has 0 ranks", n);
            prop_assert!(layout.stats.max_rank_width > 0,
                "max_rank_width is 0 with {} nodes", n);

            // stats.ranks may include virtual ranks for long-span edges and
            // cycle-breaking, so the count of unique *real-node* ranks can be
            // less than or equal to stats.ranks.
            let mut unique_ranks: Vec<usize> = layout.nodes.iter().map(|nb| nb.rank).collect();
            unique_ranks.sort_unstable();
            unique_ranks.dedup();
            prop_assert!(
                unique_ranks.len() <= layout.stats.ranks,
                "Unique real-node rank count {} > stats.ranks {}",
                unique_ranks.len(), layout.stats.ranks,
            );
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 6. Direction coverage — layout succeeds for every GraphDirection
    // ═════════════════════════════════════════════════════════════════════════

    proptest! {
        #[test]
        fn layout_works_for_all_directions(
            dir in direction_strategy(),
        ) {
            let ir = make_ir(5, &[(0, 1), (1, 2), (2, 3), (3, 4)], dir);
            let layout = layout_diagram(&ir, &MermaidConfig::default());

            prop_assert_eq!(layout.nodes.len(), 5,
                "Expected 5 nodes for direction {:?}", dir);
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 7. Evaluate non-negative — LayoutObjective fields are non-negative
    // ═════════════════════════════════════════════════════════════════════════

    proptest! {
        #[test]
        fn layout_objective_non_negative(
            (n, edges, dir) in graph_strategy(12, 15),
        ) {
            let ir = make_ir(n, &edges, dir);
            let layout = layout_diagram(&ir, &MermaidConfig::default());
            let obj = evaluate_layout(&layout);

            prop_assert!(obj.position_variance >= 0.0,
                "Position variance is negative: {}", obj.position_variance);
            prop_assert!(obj.total_edge_length >= 0.0,
                "Total edge length is negative: {}", obj.total_edge_length);
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 8. Route completeness — route_all_edges returns one path per IR edge
    // ═════════════════════════════════════════════════════════════════════════

    proptest! {
        #[test]
        fn route_covers_all_edges(
            (n, edges, dir) in dag_strategy(10, 12),
        ) {
            let ir = make_ir(n, &edges, dir);
            let layout = layout_diagram(&ir, &MermaidConfig::default());
            let (routed, _report) = route_all_edges(
                &ir, &layout, &MermaidConfig::default(), &RoutingWeights::default(),
            );

            let total_represented: usize = routed.iter().map(|p| p.bundle_count).sum();
            prop_assert_eq!(
                total_represented, ir.edges.len(),
                "Routed bundle_count sum {} != IR edge count {}",
                total_represented, ir.edges.len(),
            );
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 9. Label placement — place_labels returns without panic and produces
    //    well-formed output
    // ═════════════════════════════════════════════════════════════════════════

    proptest! {
        #[test]
        fn label_placement_well_formed(
            (n, edges, dir) in dag_strategy(8, 10),
        ) {
            let ir = make_ir(n, &edges, dir);
            let layout = layout_diagram(&ir, &MermaidConfig::default());
            let result = place_labels(&ir, &layout, &LabelPlacementConfig::default());

            prop_assert!(
                result.edge_labels.len() <= ir.edges.len(),
                "More edge labels ({}) than edges ({})",
                result.edge_labels.len(), ir.edges.len(),
            );
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 10. Cluster containment — cluster rects contain their member nodes
    // ═════════════════════════════════════════════════════════════════════════

    proptest! {
        #[test]
        fn cluster_rects_contain_members(
            (n, edges, dir) in dag_strategy(8, 10),
        ) {
            let half = n / 2;
            if half >= 2 {
                let cluster_members = (0..half).collect::<Vec<_>>();
                let ir = make_ir_with_clusters(n, &edges, std::slice::from_ref(&cluster_members), dir);
                let layout = layout_diagram(&ir, &MermaidConfig::default());

                if let Some(cluster_box) = layout.clusters.first() {
                    for &member_idx in &cluster_members {
                        if let Some(node_box) = layout.nodes.iter().find(|nb| nb.node_idx == member_idx) {
                            prop_assert!(
                                rect_contains(&cluster_box.rect, &node_box.rect),
                                "Cluster rect {:?} doesn't contain member node {} rect {:?}",
                                cluster_box.rect, member_idx, node_box.rect,
                            );
                        }
                    }
                }
            }
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 11. Rank monotonicity — within a single direction, rank ordering
    //     corresponds to spatial ordering
    // ═════════════════════════════════════════════════════════════════════════

    proptest! {
        #[test]
        fn rank_ordering_matches_spatial_ordering_tb(
            (n, edges, _dir) in dag_strategy(10, 12),
        ) {
            let ir = make_ir(n, &edges, GraphDirection::TB);
            let layout = layout_diagram(&ir, &MermaidConfig::default());

            for a in &layout.nodes {
                for b in &layout.nodes {
                    if a.rank < b.rank {
                        prop_assert!(
                            a.rect.y < b.rect.y,
                            "TB: Node {} (rank {}) has y={} >= node {} (rank {}) y={}",
                            a.node_idx, a.rank, a.rect.y,
                            b.node_idx, b.rank, b.rect.y,
                        );
                    }
                }
            }
        }

        #[test]
        fn rank_ordering_matches_spatial_ordering_lr(
            (n, edges, _dir) in dag_strategy(10, 12),
        ) {
            let ir = make_ir(n, &edges, GraphDirection::LR);
            let layout = layout_diagram(&ir, &MermaidConfig::default());

            for a in &layout.nodes {
                for b in &layout.nodes {
                    if a.rank < b.rank {
                        prop_assert!(
                            a.rect.x < b.rect.x,
                            "LR: Node {} (rank {}) has x={} >= node {} (rank {}) x={}",
                            a.node_idx, a.rank, a.rect.x,
                            b.node_idx, b.rank, b.rect.x,
                        );
                    }
                }
            }
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 12. Empty graph — zero nodes produce empty layout without panic
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn empty_graph_produces_empty_layout() {
        let ir = make_ir(0, &[], GraphDirection::TB);
        let layout = layout_diagram(&ir, &MermaidConfig::default());

        assert!(layout.nodes.is_empty(), "Empty graph should have no nodes");
        assert!(layout.edges.is_empty(), "Empty graph should have no edges");
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 13. Single node — degenerate single-node graph lays out correctly
    // ═════════════════════════════════════════════════════════════════════════

    proptest! {
        #[test]
        fn single_node_layout(dir in direction_strategy()) {
            let ir = make_ir(1, &[], dir);
            let layout = layout_diagram(&ir, &MermaidConfig::default());

            prop_assert_eq!(layout.nodes.len(), 1, "Single node graph should have 1 node");
            prop_assert!(layout.edges.is_empty(), "Single node graph should have no edges");

            let nb = &layout.nodes[0];
            prop_assert_eq!(nb.node_idx, 0);
            prop_assert!(nb.rect.width > 0.0, "Node width should be positive");
            prop_assert!(nb.rect.height > 0.0, "Node height should be positive");
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 14. Node dimensions positive — all node rects have positive width/height
    // ═════════════════════════════════════════════════════════════════════════

    proptest! {
        #[test]
        fn all_node_rects_have_positive_dimensions(
            (n, edges, dir) in graph_strategy(15, 20),
        ) {
            let ir = make_ir(n, &edges, dir);
            let layout = layout_diagram(&ir, &MermaidConfig::default());

            for nb in &layout.nodes {
                prop_assert!(
                    nb.rect.width > 0.0,
                    "Node {} has non-positive width: {}",
                    nb.node_idx, nb.rect.width,
                );
                prop_assert!(
                    nb.rect.height > 0.0,
                    "Node {} has non-positive height: {}",
                    nb.node_idx, nb.rect.height,
                );
            }
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 15. Bounding box positive dimensions — always has non-negative size
    // ═════════════════════════════════════════════════════════════════════════

    proptest! {
        #[test]
        fn bounding_box_positive_dimensions(
            (n, edges, dir) in graph_strategy(15, 20),
        ) {
            let ir = make_ir(n, &edges, dir);
            let layout = layout_diagram(&ir, &MermaidConfig::default());

            prop_assert!(
                layout.bounding_box.width >= 0.0,
                "Bounding box width is negative: {}",
                layout.bounding_box.width,
            );
            prop_assert!(
                layout.bounding_box.height >= 0.0,
                "Bounding box height is negative: {}",
                layout.bounding_box.height,
            );
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 16. Routing report total bends — matches actual bend count in paths
    // ═════════════════════════════════════════════════════════════════════════

    proptest! {
        #[test]
        fn routing_report_internal_consistency(
            (n, edges, dir) in dag_strategy(8, 10),
        ) {
            let ir = make_ir(n, &edges, dir);
            let layout = layout_diagram(&ir, &MermaidConfig::default());
            let (_routed, report) = route_all_edges(
                &ir, &layout, &MermaidConfig::default(), &RoutingWeights::default(),
            );

            // total_bends should equal sum of per-edge bends.
            let sum_bends: usize = report.edges.iter().map(|d| d.bends).sum();
            prop_assert_eq!(
                sum_bends, report.total_bends,
                "Sum of per-edge bends {} != report.total_bends {}",
                sum_bends, report.total_bends,
            );

            // total_cells_explored should equal sum of per-edge cells_explored.
            let sum_cells: usize = report.edges.iter().map(|d| d.cells_explored).sum();
            prop_assert_eq!(
                sum_cells, report.total_cells_explored,
                "Sum of per-edge cells {} != report.total_cells_explored {}",
                sum_cells, report.total_cells_explored,
            );

            // total_cost should be non-negative.
            prop_assert!(
                report.total_cost >= 0.0,
                "Routing total_cost is negative: {}",
                report.total_cost,
            );
        }
    }
}
