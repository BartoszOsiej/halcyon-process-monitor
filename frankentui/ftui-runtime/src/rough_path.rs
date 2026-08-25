//! Rough-path signatures for sequential trace feature extraction.
//!
//! Rough-path signatures are universal noncommutative features extracted from
//! sequential data (frame time series, event streams, syscall traces). They
//! capture the "shape" of a path regardless of parameterization, making them
//! ideal for anomaly detection and workload characterization.
//!
//! # Mathematical Model
//!
//! Given a d-dimensional path `X: [0,T] → ℝ^d`, the **signature** is the
//! collection of iterated integrals:
//!
//! ```text
//! S(X)^{i₁,...,iₖ} = ∫₀<t₁<...<tₖ<T dX^{i₁}_{t₁} ⊗ ... ⊗ dX^{iₖ}_{tₖ}
//! ```
//!
//! The **truncated signature** at depth N keeps only terms with k ≤ N.
//! For practical use, depth 3-4 captures sufficient structure.
//!
//! # Discretization
//!
//! For discrete time series X = (x₀, x₁, ..., x_n), the increments are
//! Δx_i = x_{i+1} - x_i, and the iterated integrals become iterated sums:
//!
//! ```text
//! S^{i}     = Σ Δx^i_k                      (depth 1)
//! S^{i,j}   = Σ_{k<l} Δx^i_k · Δx^j_l      (depth 2)
//! S^{i,j,m} = Σ_{k<l<r} Δx^i_k · Δx^j_l · Δx^m_r  (depth 3)
//! ```
//!
//! # Efficient Computation (Chen's Identity)
//!
//! Rather than brute-force triple loops, we use Chen's identity for
//! incremental computation:
//!
//! ```text
//! S(X_{[0,t+1]}) = S(X_{[0,t]}) ⊗ S(Δx_{t})
//! ```
//!
//! where ⊗ is the tensor (shuffle) product. This gives O(n · d^N) instead
//! of O(n^N · d^N).
//!
//! # Usage
//!
//! ```rust
//! use ftui_runtime::rough_path::SignatureExtractor;
//!
//! // 2D path: (frame_time_ms, alloc_count)
//! let mut ext = SignatureExtractor::new(2, 3); // dim=2, depth=3
//!
//! ext.observe(&[16.0, 1200.0]);
//! ext.observe(&[17.0, 1250.0]);
//! ext.observe(&[15.0, 1180.0]);
//! ext.observe(&[32.0, 1500.0]); // spike!
//!
//! let sig = ext.signature();
//! // sig contains depth-1, depth-2, and depth-3 terms
//! // Use for anomaly detection, distance computation, etc.
//! ```
//!
//! # Applications
//!
//! | Use case | Dimensions | What it detects |
//! |----------|-----------|-----------------|
//! | Frame timing | 1D (ms) | Stutter patterns, periodicity |
//! | Render+alloc | 2D (ms, bytes) | Correlated regressions |
//! | Event stream | 3D (dt, type, payload) | Workload fingerprints |
//!
//! # Fallback
//!
//! When signatures are too expensive (very high-dimensional data), use
//! standard statistical features (mean, variance, skew, autocorrelation).

/// Configuration for signature extraction.
#[derive(Debug, Clone)]
pub struct SignatureConfig {
    /// Number of dimensions in the input path.
    pub dim: usize,
    /// Maximum depth of the truncated signature (typically 2-4).
    pub max_depth: usize,
}

/// Computes the number of signature components for given dim and depth.
///
/// Total terms = Σ_{k=1}^{depth} dim^k = dim * (dim^depth - 1) / (dim - 1)
/// For dim=1, it's just `depth`.
pub fn signature_size(dim: usize, max_depth: usize) -> usize {
    if dim == 0 || max_depth == 0 {
        return 0;
    }
    if dim == 1 {
        return max_depth;
    }
    let mut total: usize = 0;
    let mut power: usize = 1;
    for _ in 0..max_depth {
        power = power.saturating_mul(dim);
        total = total.saturating_add(power);
    }
    total
}

/// Incremental truncated signature extractor.
///
/// Maintains the running signature of a d-dimensional discrete path,
/// updated incrementally via Chen's identity.
#[derive(Debug, Clone)]
pub struct SignatureExtractor {
    dim: usize,
    max_depth: usize,
    /// Flattened signature terms, organized by depth.
    /// Depth k has dim^k terms, stored in row-major order of multi-indices.
    terms: Vec<f64>,
    /// Offsets into `terms` for each depth level.
    offsets: Vec<usize>,
    /// Number of observations seen.
    count: usize,
    /// Last observed point (for computing increments).
    last: Option<Vec<f64>>,
}

impl SignatureExtractor {
    /// Create a new extractor for `dim`-dimensional paths at truncation `max_depth`.
    pub fn new(dim: usize, max_depth: usize) -> Self {
        let total = signature_size(dim, max_depth);
        let mut offsets = Vec::with_capacity(max_depth + 1);
        offsets.push(0);
        let mut power = 1;
        for _ in 0..max_depth {
            power *= dim;
            offsets.push(offsets.last().unwrap() + power);
        }
        Self {
            dim,
            max_depth,
            terms: vec![0.0; total],
            offsets,
            count: 0,
            last: None,
        }
    }

    /// Number of dimensions.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Maximum signature depth.
    pub fn max_depth(&self) -> usize {
        self.max_depth
    }

    /// Number of observations processed.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Total number of signature terms.
    pub fn num_terms(&self) -> usize {
        self.terms.len()
    }

    /// Observe a new point on the path.
    ///
    /// `point` must have length `dim`. Panics if the length is wrong.
    /// The first observation establishes the baseline; increments start
    /// from the second observation onward.
    pub fn observe(&mut self, point: &[f64]) {
        assert_eq!(
            point.len(),
            self.dim,
            "point dimension {} != extractor dimension {}",
            point.len(),
            self.dim
        );

        self.count += 1;

        if let Some(ref last) = self.last {
            // Compute increment.
            let dx: Vec<f64> = point.iter().zip(last.iter()).map(|(a, b)| a - b).collect();
            // Update signature via Chen's identity (incremental).
            self.extend_signature(&dx);
        }

        self.last = Some(point.to_vec());
    }

    /// Extend the running signature by one increment using Chen's identity.
    ///
    /// For each depth k from `max_depth` down to 1:
    ///   S^{i₁,...,iₖ} += S^{i₁,...,i_{k-1}} · Δx^{iₖ}
    ///
    /// Processing from highest depth down prevents double-counting.
    fn extend_signature(&mut self, dx: &[f64]) {
        // Process depths from highest to lowest to avoid using updated lower
        // depths in the same step.
        for depth in (1..=self.max_depth).rev() {
            if depth == 1 {
                // Depth 1: S^i += dx^i
                for (term, &dx_val) in self.terms[..self.dim].iter_mut().zip(dx.iter()) {
                    *term += dx_val;
                }
            } else {
                // Depth k > 1: S^{...,i} += S^{...} * dx^i
                // Parent has dim^(k-1) terms at offset[k-2]..offset[k-1]
                let parent_start = self.offsets[depth - 2];
                let parent_count = self.offsets[depth - 1] - parent_start;
                let child_start = self.offsets[depth - 1];

                // Read parent values first (snapshot to avoid borrow issues).
                let parents: Vec<f64> =
                    self.terms[parent_start..parent_start + parent_count].to_vec();

                for (p_idx, &parent_val) in parents.iter().enumerate() {
                    for (d_idx, &dx_val) in dx.iter().enumerate() {
                        let child_idx = child_start + p_idx * self.dim + d_idx;
                        self.terms[child_idx] += parent_val * dx_val;
                    }
                }
            }
        }
    }

    /// Get the current signature as a slice.
    pub fn signature(&self) -> &[f64] {
        &self.terms
    }

    /// Get signature terms at a specific depth (1-indexed).
    ///
    /// Returns `None` if depth is 0 or exceeds `max_depth`.
    pub fn signature_at_depth(&self, depth: usize) -> Option<&[f64]> {
        if depth == 0 || depth > self.max_depth {
            return None;
        }
        let start = self.offsets[depth - 1];
        let end = self.offsets[depth];
        Some(&self.terms[start..end])
    }

    /// L2 norm of the signature vector.
    pub fn norm(&self) -> f64 {
        self.terms.iter().map(|x| x * x).sum::<f64>().sqrt()
    }

    /// Reset the extractor to its initial state.
    pub fn reset(&mut self) {
        self.terms.fill(0.0);
        self.count = 0;
        self.last = None;
    }
}

/// Compute the L2 distance between two signature vectors.
///
/// The signatures must have the same length.
pub fn signature_distance(a: &[f64], b: &[f64]) -> f64 {
    assert_eq!(a.len(), b.len(), "signature length mismatch");
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y) * (x - y))
        .sum::<f64>()
        .sqrt()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_size_basic() {
        assert_eq!(signature_size(0, 3), 0);
        assert_eq!(signature_size(3, 0), 0);
        assert_eq!(signature_size(1, 3), 3); // 1 + 1 + 1
        assert_eq!(signature_size(2, 1), 2); // 2
        assert_eq!(signature_size(2, 2), 6); // 2 + 4
        assert_eq!(signature_size(2, 3), 14); // 2 + 4 + 8
        assert_eq!(signature_size(3, 2), 12); // 3 + 9
    }

    #[test]
    fn extractor_initial_state() {
        let ext = SignatureExtractor::new(2, 3);
        assert_eq!(ext.dim(), 2);
        assert_eq!(ext.max_depth(), 3);
        assert_eq!(ext.count(), 0);
        assert_eq!(ext.num_terms(), 14); // 2 + 4 + 8
        assert!(ext.signature().iter().all(|&x| x == 0.0));
    }

    #[test]
    fn single_observation_no_signature() {
        let mut ext = SignatureExtractor::new(2, 2);
        ext.observe(&[1.0, 2.0]);
        // Only one point — no increment, signature stays zero.
        assert!(ext.signature().iter().all(|&x| x == 0.0));
        assert_eq!(ext.count(), 1);
    }

    #[test]
    fn depth1_is_total_increment() {
        let mut ext = SignatureExtractor::new(2, 2);
        ext.observe(&[0.0, 0.0]);
        ext.observe(&[1.0, 3.0]);
        ext.observe(&[4.0, 5.0]);

        // Depth 1: total increments = (1,3) + (3,2) = (4, 5)
        let d1 = ext.signature_at_depth(1).unwrap();
        assert!((d1[0] - 4.0).abs() < 1e-10);
        assert!((d1[1] - 5.0).abs() < 1e-10);
    }

    #[test]
    fn depth2_iterated_integrals() {
        let mut ext = SignatureExtractor::new(1, 2);
        // 1D path: 0 → 1 → 3 → 6
        // Increments: 1, 2, 3
        ext.observe(&[0.0]);
        ext.observe(&[1.0]);
        ext.observe(&[3.0]);
        ext.observe(&[6.0]);

        // Depth 1: S^1 = 1 + 2 + 3 = 6
        let d1 = ext.signature_at_depth(1).unwrap();
        assert!((d1[0] - 6.0).abs() < 1e-10);

        // Depth 2: S^{1,1} = Σ_{k<l} Δx_k · Δx_l
        // = 1*2 + 1*3 + 2*3 = 2 + 3 + 6 = 11
        let d2 = ext.signature_at_depth(2).unwrap();
        assert!((d2[0] - 11.0).abs() < 1e-10);
    }

    #[test]
    fn signature_depth_out_of_range() {
        let ext = SignatureExtractor::new(2, 3);
        assert!(ext.signature_at_depth(0).is_none());
        assert!(ext.signature_at_depth(4).is_none());
        assert!(ext.signature_at_depth(1).is_some());
        assert!(ext.signature_at_depth(3).is_some());
    }

    #[test]
    fn norm_computation() {
        let mut ext = SignatureExtractor::new(1, 1);
        ext.observe(&[0.0]);
        ext.observe(&[3.0]);
        // Depth 1: S^1 = 3
        assert!((ext.norm() - 3.0).abs() < 1e-10);
    }

    #[test]
    fn distance_computation() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 6.0, 3.0];
        // distance = sqrt(9 + 16 + 0) = 5
        assert!((signature_distance(&a, &b) - 5.0).abs() < 1e-10);
    }

    #[test]
    fn reset() {
        let mut ext = SignatureExtractor::new(2, 2);
        ext.observe(&[0.0, 0.0]);
        ext.observe(&[1.0, 1.0]);
        assert!(ext.norm() > 0.0);
        ext.reset();
        assert_eq!(ext.count(), 0);
        assert!(ext.signature().iter().all(|&x| x == 0.0));
    }

    #[test]
    fn two_dim_depth3() {
        let mut ext = SignatureExtractor::new(2, 3);
        ext.observe(&[0.0, 0.0]);
        ext.observe(&[1.0, 0.0]);
        ext.observe(&[1.0, 1.0]);

        // Increments: (1,0), (0,1)
        // Depth 1: (1, 1)
        let d1 = ext.signature_at_depth(1).unwrap();
        assert!((d1[0] - 1.0).abs() < 1e-10);
        assert!((d1[1] - 1.0).abs() < 1e-10);

        // Depth 2 (2x2 = 4 terms): S^{ij} = Σ_{k<l} Δx^i_k · Δx^j_l
        // S^{1,1} = 1*0 = 0
        // S^{1,2} = 1*1 = 1
        // S^{2,1} = 0*0 = 0
        // S^{2,2} = 0*1 = 0
        let d2 = ext.signature_at_depth(2).unwrap();
        assert!((d2[0] - 0.0).abs() < 1e-10); // S^{1,1}
        assert!((d2[1] - 1.0).abs() < 1e-10); // S^{1,2}
        assert!((d2[2] - 0.0).abs() < 1e-10); // S^{2,1}
        assert!((d2[3] - 0.0).abs() < 1e-10); // S^{2,2}
    }

    #[test]
    fn translation_invariance() {
        // Signature should be the same regardless of starting point.
        let mut ext1 = SignatureExtractor::new(2, 2);
        ext1.observe(&[0.0, 0.0]);
        ext1.observe(&[1.0, 2.0]);
        ext1.observe(&[3.0, 1.0]);

        let mut ext2 = SignatureExtractor::new(2, 2);
        ext2.observe(&[100.0, 200.0]);
        ext2.observe(&[101.0, 202.0]);
        ext2.observe(&[103.0, 201.0]);

        let s1 = ext1.signature();
        let s2 = ext2.signature();
        for (a, b) in s1.iter().zip(s2.iter()) {
            assert!(
                (a - b).abs() < 1e-10,
                "Translation invariance violated: {} != {}",
                a,
                b
            );
        }
    }

    #[test]
    fn constant_path_zero_signature() {
        // If all points are the same, all increments are zero.
        let mut ext = SignatureExtractor::new(3, 3);
        for _ in 0..10 {
            ext.observe(&[5.0, 5.0, 5.0]);
        }
        assert!(ext.signature().iter().all(|&x| x == 0.0));
    }

    #[test]
    fn signature_distance_zero_for_same() {
        let a = vec![1.0, 2.0, 3.0];
        assert!((signature_distance(&a, &a)).abs() < 1e-15);
    }

    #[test]
    #[should_panic(expected = "point dimension")]
    fn wrong_dimension_panics() {
        let mut ext = SignatureExtractor::new(2, 2);
        ext.observe(&[1.0]); // wrong: expected 2
    }

    #[test]
    fn frame_timing_anomaly_detection() {
        // Simulate: normal frame times ~16ms, then anomalous spike
        let mut normal = SignatureExtractor::new(1, 3);
        for &t in &[16.0, 16.1, 15.9, 16.0, 16.2, 15.8, 16.0, 16.1] {
            normal.observe(&[t]);
        }

        let mut anomalous = SignatureExtractor::new(1, 3);
        for &t in &[16.0, 16.1, 15.9, 64.0, 16.0, 16.0, 16.0, 16.0] {
            anomalous.observe(&[t]);
        }

        // The anomalous path should have a significantly different signature
        let dist = signature_distance(normal.signature(), anomalous.signature());
        assert!(
            dist > 1.0,
            "Anomalous path should be far from normal (dist={})",
            dist
        );
    }
}
