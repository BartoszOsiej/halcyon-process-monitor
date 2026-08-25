//! Elias-Fano encoding for monotone integer sequences.
//!
//! Provides space-efficient representation of sorted non-decreasing sequences
//! with O(1) access, rank, and select operations. Designed for height prefix
//! sums in virtualized lists, enabling "which row is at pixel offset Y?"
//! queries with ~10x less memory than a dense `Vec<u64>`.
//!
//! # Encoding
//!
//! Each value is split into low and high parts:
//! - **Low bits**: `l = floor(log2(U/n))` bits per element, stored in a packed
//!   dense array.
//! - **High bits**: stored in unary encoding via a bitvector supporting rank
//!   and select. The bitvector has `n` one-bits (one per element) and at most
//!   `U >> l` zero-bits (gap separators).
//!
//! Rank superblocks are precomputed every 512 bits for O(1) rank queries.
//!
//! # Example
//!
//! ```
//! use ftui_widgets::elias_fano::EliasFano;
//!
//! let heights = [10, 25, 40, 55, 100];
//! let ef = EliasFano::encode(&heights);
//!
//! assert_eq!(ef.len(), 5);
//! assert_eq!(ef.access(0), 10);
//! assert_eq!(ef.access(4), 100);
//!
//! // Which row is at pixel 30? → rank(30) = 2 (rows 0,1 have offsets ≤ 30)
//! assert_eq!(ef.rank(30), 2);
//!
//! // First row with offset ≥ 30?
//! assert_eq!(ef.next_geq(30), Some((2, 40)));
//! ```

/// Number of `u64` words per rank superblock (512 bits).
const SUPERBLOCK_WORDS: usize = 8;

/// Elias-Fano encoded monotone integer sequence.
///
/// Supports O(1) access, rank, select, and next_geq queries on a sorted
/// non-decreasing sequence of `u64` values.
#[derive(Debug, Clone)]
pub struct EliasFano {
    /// Low bits packed contiguously (`low_width` bits per element).
    low_bits: Vec<u64>,
    /// High bits in unary encoding (1-bit per element, 0-bits as gap separators).
    high_bits: Vec<u64>,
    /// Cumulative popcount at superblock boundaries.
    /// `rank_superblocks[i]` = number of 1-bits in `high_bits[0..i*SUPERBLOCK_WORDS]`.
    rank_superblocks: Vec<u64>,
    /// Number of encoded elements.
    n: usize,
    /// Bits per low part.
    low_width: u32,
    /// Maximum value in the sequence (universe upper bound).
    universe: u64,
}

impl EliasFano {
    /// Encode a sorted non-decreasing sequence of values.
    ///
    /// # Panics
    ///
    /// Panics if the sequence is not monotonically non-decreasing.
    pub fn encode(values: &[u64]) -> Self {
        let n = values.len();
        if n == 0 {
            return Self {
                low_bits: Vec::new(),
                high_bits: Vec::new(),
                rank_superblocks: vec![0],
                n: 0,
                low_width: 0,
                universe: 0,
            };
        }

        // Verify monotonicity
        for i in 1..n {
            assert!(
                values[i] >= values[i - 1],
                "sequence must be non-decreasing: values[{}]={} < values[{}]={}",
                i,
                values[i],
                i - 1,
                values[i - 1]
            );
        }

        let universe = values[n - 1];

        // Compute low_width: floor(log2(universe / n)), minimum 0
        let low_width = if universe == 0 || n <= 1 {
            0
        } else {
            let ratio = universe / n as u64;
            if ratio == 0 { 0 } else { ratio.ilog2() }
        };

        let low_mask = if low_width == 0 {
            0
        } else if low_width >= 64 {
            u64::MAX
        } else {
            (1u64 << low_width) - 1
        };

        // Allocate low bits
        let total_low_bits = n as u64 * low_width as u64;
        let low_words = div_ceil_u64(total_low_bits, 64) as usize;
        let mut low_bits = vec![0u64; low_words];

        // Allocate high bits
        let max_high = universe >> low_width;
        let high_bit_len = n as u64 + max_high + 1;
        let high_words = div_ceil_u64(high_bit_len, 64) as usize;
        let mut high_bits = vec![0u64; high_words];

        // Encode each value
        for (i, &v) in values.iter().enumerate() {
            // Store low bits
            if low_width > 0 {
                let low = v & low_mask;
                set_bits(&mut low_bits, i as u64 * low_width as u64, low_width, low);
            }

            // Store high bit: set bit at position (high_part + i) in high_bits
            let high = v >> low_width;
            let bit_pos = high + i as u64;
            let word_idx = (bit_pos / 64) as usize;
            let bit_offset = bit_pos % 64;
            high_bits[word_idx] |= 1u64 << bit_offset;
        }

        // Build rank superblocks
        let num_superblocks = div_ceil(high_words, SUPERBLOCK_WORDS);
        let mut rank_superblocks = Vec::with_capacity(num_superblocks + 1);
        rank_superblocks.push(0u64);
        let mut cumulative = 0u64;
        for chunk in high_bits.chunks(SUPERBLOCK_WORDS) {
            for &word in chunk {
                cumulative += word.count_ones() as u64;
            }
            rank_superblocks.push(cumulative);
        }

        Self {
            low_bits,
            high_bits,
            rank_superblocks,
            n,
            low_width,
            universe,
        }
    }

    /// Access the value at the given index.
    ///
    /// Reconstructs value `i` from its low and high parts.
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.len()`.
    pub fn access(&self, index: usize) -> u64 {
        assert!(
            index < self.n,
            "index {index} out of bounds (len={})",
            self.n
        );

        let low = if self.low_width > 0 {
            get_bits(
                &self.low_bits,
                index as u64 * self.low_width as u64,
                self.low_width,
            )
        } else {
            0
        };

        // High part: find position of the index-th 1-bit in high_bits,
        // then high = position - index
        let pos = self.select1(index);
        let high = pos as u64 - index as u64;

        (high << self.low_width) | low
    }

    /// Number of elements with value ≤ `value`.
    ///
    /// Returns 0 if all elements are greater than `value`.
    /// Returns `n` if all elements are ≤ `value`.
    pub fn rank(&self, value: u64) -> usize {
        if self.n == 0 || value < self.access(0) {
            return 0;
        }
        if value >= self.universe {
            return self.n;
        }

        let high = value >> self.low_width;
        let low = if self.low_width > 0 {
            value & ((1u64 << self.low_width) - 1)
        } else {
            0
        };

        // Find elements in the bucket for this high value.
        // In the high_bits bitvector, the 0-bits separate high values.
        // select0(high) gives the position after the (high-1) bucket separator.
        // Elements in bucket `high` are the 1-bits between select0(high) and select0(high+1).

        // Start of bucket: position after the high-th 0-bit
        // If high == 0, start scanning from position 0
        let bucket_start_pos = if high == 0 {
            0
        } else {
            self.select0(high as usize - 1) + 1
        };

        // End of bucket: position of the (high+1)-th 0-bit (exclusive)
        // The 0-bit at position select0(high) marks the end of bucket `high`
        let bucket_end_pos = if (high as usize) < self.high_bits.len() * 64 {
            // Find the next 0-bit at or after high position
            match self.try_select0(high as usize) {
                Some(pos) => pos,
                None => self.high_bits.len() * 64,
            }
        } else {
            self.high_bits.len() * 64
        };

        // Count 1-bits in [0, bucket_start_pos) = elements with high part < high
        let base_rank = self.rank1(bucket_start_pos);

        // Scan elements in this bucket to find those with low part ≤ low
        let mut count = base_rank;
        let mut pos = bucket_start_pos;
        while pos < bucket_end_pos {
            let word_idx = pos / 64;
            if word_idx >= self.high_bits.len() {
                break;
            }
            let bit = pos % 64;
            let word = self.high_bits[word_idx] >> bit;
            if word & 1 == 1 {
                // This is a 1-bit, meaning an element
                let elem_idx = count;
                if elem_idx >= self.n {
                    break;
                }
                let elem_low = if self.low_width > 0 {
                    get_bits(
                        &self.low_bits,
                        elem_idx as u64 * self.low_width as u64,
                        self.low_width,
                    )
                } else {
                    0
                };
                if elem_low <= low {
                    count += 1;
                } else {
                    break;
                }
            } else {
                // 0-bit = end of bucket
                break;
            }
            pos += 1;
        }

        count
    }

    /// Get the value at the given rank (0-indexed).
    ///
    /// Equivalent to `access(rank)`.
    ///
    /// # Panics
    ///
    /// Panics if `rank >= self.len()`.
    pub fn select(&self, rank: usize) -> u64 {
        self.access(rank)
    }

    /// Find the first element ≥ `value`.
    ///
    /// Returns `Some((index, element_value))` or `None` if all elements are less
    /// than `value`.
    pub fn next_geq(&self, value: u64) -> Option<(usize, u64)> {
        if self.n == 0 {
            return None;
        }
        if value == 0 {
            return Some((0, self.access(0)));
        }
        if value > self.universe {
            return None;
        }

        // Count of elements strictly less than value = rank(value - 1)
        let idx = self.rank(value - 1);
        if idx >= self.n {
            return None;
        }

        Some((idx, self.access(idx)))
    }

    /// Number of encoded elements.
    #[inline]
    pub fn len(&self) -> usize {
        self.n
    }

    /// Whether the encoded sequence is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Total memory usage in bytes (excluding struct overhead).
    pub fn size_in_bytes(&self) -> usize {
        self.low_bits.len() * 8 + self.high_bits.len() * 8 + self.rank_superblocks.len() * 8
    }

    /// Information-theoretic minimum bytes for this sequence.
    pub fn optimal_size_in_bytes(&self) -> usize {
        if self.n == 0 {
            return 0;
        }
        // Information-theoretic minimum: log2(C(universe + n, n)) / 8
        // Approximation: n * log2(universe / n) / 8 + n / 4
        let bits_per_elem = if self.universe > 0 && self.n > 1 {
            let ratio = self.universe as f64 / self.n as f64;
            ratio.log2().max(0.0) + 2.0
        } else {
            2.0
        };
        ((self.n as f64 * bits_per_elem) / 8.0).ceil() as usize
    }

    // ── Bitvector primitives ────────────────────────────────────────

    /// Count 1-bits in `high_bits[0..pos)`.
    fn rank1(&self, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }
        let word_idx = pos / 64;
        let bit_idx = pos % 64;

        // Superblock contribution
        let sb_idx = word_idx / SUPERBLOCK_WORDS;
        let mut count = self.rank_superblocks[sb_idx] as usize;

        // Count full words within the superblock
        let sb_start = sb_idx * SUPERBLOCK_WORDS;
        for i in sb_start..word_idx.min(self.high_bits.len()) {
            count += self.high_bits[i].count_ones() as usize;
        }

        // Partial word
        if bit_idx > 0 && word_idx < self.high_bits.len() {
            let mask = (1u64 << bit_idx) - 1;
            count += (self.high_bits[word_idx] & mask).count_ones() as usize;
        }

        count
    }

    /// Find position of the `k`-th 1-bit (0-indexed).
    fn select1(&self, k: usize) -> usize {
        assert!(k < self.n, "select1({k}) out of bounds (n={})", self.n);

        // Binary search on superblocks
        let target = k as u64;
        let mut lo = 0usize;
        let mut hi = self.rank_superblocks.len() - 1;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.rank_superblocks[mid + 1] <= target {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        let sb = lo;
        let mut remaining = k - self.rank_superblocks[sb] as usize;
        let word_start = sb * SUPERBLOCK_WORDS;

        // Linear scan within superblock
        for w in word_start..self.high_bits.len() {
            let ones = self.high_bits[w].count_ones() as usize;
            if remaining < ones {
                // The target 1-bit is within this word
                return w * 64 + select_in_word(self.high_bits[w], remaining);
            }
            remaining -= ones;
        }

        unreachable!("select1({k}): not enough 1-bits in high_bits")
    }

    /// Find position of the `k`-th 0-bit (0-indexed).
    fn select0(&self, k: usize) -> usize {
        self.try_select0(k)
            .unwrap_or_else(|| panic!("select0({k}): not enough 0-bits in high_bits"))
    }

    /// Try to find position of the `k`-th 0-bit (0-indexed).
    fn try_select0(&self, k: usize) -> Option<usize> {
        // Binary search on superblocks using zero counts.
        // zeros in [0, sb*SUPERBLOCK_WORDS*64) = sb*SUPERBLOCK_WORDS*64 - rank_superblocks[sb]
        let mut lo = 0usize;
        let mut hi = self.rank_superblocks.len() - 1;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let total_bits = (mid + 1) * SUPERBLOCK_WORDS * 64;
            let ones = self.rank_superblocks[mid + 1] as usize;
            let zeros = total_bits - ones;
            if zeros <= k {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        let sb = lo;
        let sb_total_bits = sb * SUPERBLOCK_WORDS * 64;
        let sb_ones = self.rank_superblocks[sb] as usize;
        let mut remaining = k - (sb_total_bits - sb_ones);
        let word_start = sb * SUPERBLOCK_WORDS;

        for w in word_start..self.high_bits.len() {
            let zeros = self.high_bits[w].count_zeros() as usize;
            if remaining < zeros {
                return Some(w * 64 + select0_in_word(self.high_bits[w], remaining));
            }
            remaining -= zeros;
        }

        None
    }
}

// ── Bit manipulation helpers ────────────────────────────────────────

/// Extract `width` bits from a packed bit array starting at `bit_pos`.
fn get_bits(words: &[u64], bit_pos: u64, width: u32) -> u64 {
    if width == 0 {
        return 0;
    }
    let word_idx = (bit_pos / 64) as usize;
    let bit_offset = (bit_pos % 64) as u32;
    let mask = if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    };

    if bit_offset + width <= 64 {
        (words[word_idx] >> bit_offset) & mask
    } else {
        let lo = words[word_idx] >> bit_offset;
        let hi = words[word_idx + 1] << (64 - bit_offset);
        (lo | hi) & mask
    }
}

/// Store `width` bits into a packed bit array at `bit_pos`.
fn set_bits(words: &mut [u64], bit_pos: u64, width: u32, value: u64) {
    if width == 0 {
        return;
    }
    let word_idx = (bit_pos / 64) as usize;
    let bit_offset = (bit_pos % 64) as u32;
    let mask = if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    };
    let value = value & mask;

    words[word_idx] &= !(mask << bit_offset);
    words[word_idx] |= value << bit_offset;

    if bit_offset + width > 64 {
        let overflow = bit_offset + width - 64;
        let overflow_mask = if overflow >= 64 {
            u64::MAX
        } else {
            (1u64 << overflow) - 1
        };
        words[word_idx + 1] &= !overflow_mask;
        words[word_idx + 1] |= value >> (64 - bit_offset);
    }
}

/// Find the position of the `k`-th 1-bit within a u64 word (0-indexed).
fn select_in_word(word: u64, k: usize) -> usize {
    let mut remaining = k;
    let mut w = word;
    for bit in 0..64 {
        if w & 1 == 1 {
            if remaining == 0 {
                return bit;
            }
            remaining -= 1;
        }
        w >>= 1;
        if w == 0 {
            break;
        }
    }
    unreachable!("select_in_word: not enough 1-bits")
}

/// Find the position of the `k`-th 0-bit within a u64 word (0-indexed).
fn select0_in_word(word: u64, k: usize) -> usize {
    let mut remaining = k;
    let inverted = !word;
    let mut w = inverted;
    for bit in 0..64 {
        if w & 1 == 1 {
            if remaining == 0 {
                return bit;
            }
            remaining -= 1;
        }
        w >>= 1;
    }
    unreachable!("select0_in_word: not enough 0-bits")
}

/// Integer division rounding up.
fn div_ceil(a: usize, b: usize) -> usize {
    a.div_ceil(b)
}

/// Integer division rounding up for u64.
fn div_ceil_u64(a: u64, b: u64) -> u64 {
    a.div_ceil(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // ── Basic encoding/decoding ─────────────────────────────────────

    #[test]
    fn empty_sequence() {
        let ef = EliasFano::encode(&[]);
        assert_eq!(ef.len(), 0);
        assert!(ef.is_empty());
        assert_eq!(ef.rank(0), 0);
        assert_eq!(ef.rank(100), 0);
        assert_eq!(ef.next_geq(0), None);
    }

    #[test]
    fn single_element() {
        let ef = EliasFano::encode(&[42]);
        assert_eq!(ef.len(), 1);
        assert_eq!(ef.access(0), 42);
        assert_eq!(ef.select(0), 42);
        assert_eq!(ef.rank(41), 0);
        assert_eq!(ef.rank(42), 1);
        assert_eq!(ef.rank(100), 1);
        assert_eq!(ef.next_geq(0), Some((0, 42)));
        assert_eq!(ef.next_geq(42), Some((0, 42)));
        assert_eq!(ef.next_geq(43), None);
    }

    #[test]
    fn all_zeros() {
        let ef = EliasFano::encode(&[0, 0, 0, 0]);
        assert_eq!(ef.len(), 4);
        for i in 0..4 {
            assert_eq!(ef.access(i), 0);
        }
        assert_eq!(ef.rank(0), 4);
        assert_eq!(ef.next_geq(0), Some((0, 0)));
        assert_eq!(ef.next_geq(1), None);
    }

    #[test]
    fn consecutive_values() {
        let values: Vec<u64> = (0..100).collect();
        let ef = EliasFano::encode(&values);
        assert_eq!(ef.len(), 100);
        for (i, &v) in values.iter().enumerate() {
            assert_eq!(ef.access(i), v, "access({i})");
        }
    }

    #[test]
    fn height_prefix_sums() {
        // Simulate cumulative heights: [20, 45, 75, 100, 130, 180]
        let sums = [20u64, 45, 75, 100, 130, 180];
        let ef = EliasFano::encode(&sums);

        for (i, &v) in sums.iter().enumerate() {
            assert_eq!(ef.access(i), v, "access({i})");
        }

        // rank(50) = how many rows have offset ≤ 50? → 2 (rows 0,1)
        assert_eq!(ef.rank(50), 2);

        // rank(75) = 3
        assert_eq!(ef.rank(75), 3);

        // next_geq(50) → first row with offset ≥ 50 → (2, 75)
        assert_eq!(ef.next_geq(50), Some((2, 75)));
    }

    #[test]
    fn large_values() {
        let values = [1_000_000u64, 2_000_000, 3_000_000, 4_000_000];
        let ef = EliasFano::encode(&values);
        for (i, &v) in values.iter().enumerate() {
            assert_eq!(ef.access(i), v);
        }
    }

    #[test]
    fn duplicate_values() {
        let values = [5u64, 5, 10, 10, 10, 20];
        let ef = EliasFano::encode(&values);
        for (i, &v) in values.iter().enumerate() {
            assert_eq!(ef.access(i), v, "access({i})");
        }
        assert_eq!(ef.rank(5), 2);
        assert_eq!(ef.rank(10), 5);
        assert_eq!(ef.rank(9), 2);
    }

    #[test]
    fn rank_boundary_cases() {
        let values = [10u64, 20, 30, 40, 50];
        let ef = EliasFano::encode(&values);

        assert_eq!(ef.rank(0), 0);
        assert_eq!(ef.rank(9), 0);
        assert_eq!(ef.rank(10), 1);
        assert_eq!(ef.rank(15), 1);
        assert_eq!(ef.rank(50), 5);
        assert_eq!(ef.rank(100), 5);
    }

    #[test]
    fn next_geq_exhaustive() {
        let values = [10u64, 20, 30, 40, 50];
        let ef = EliasFano::encode(&values);

        assert_eq!(ef.next_geq(0), Some((0, 10)));
        assert_eq!(ef.next_geq(10), Some((0, 10)));
        assert_eq!(ef.next_geq(11), Some((1, 20)));
        assert_eq!(ef.next_geq(50), Some((4, 50)));
        assert_eq!(ef.next_geq(51), None);
    }

    #[test]
    fn select_matches_access() {
        let values = [3u64, 7, 15, 31, 63, 127, 255];
        let ef = EliasFano::encode(&values);
        for i in 0..values.len() {
            assert_eq!(ef.select(i), ef.access(i));
        }
    }

    #[test]
    fn space_efficiency() {
        // 10K elements with universe ~1M
        let values: Vec<u64> = (0..10_000).map(|i| i * 100).collect();
        let ef = EliasFano::encode(&values);

        let dense_size = values.len() * 8; // 80KB
        let ef_size = ef.size_in_bytes();

        assert!(
            ef_size < dense_size,
            "Elias-Fano ({ef_size} bytes) should be smaller than dense ({dense_size} bytes)"
        );
    }

    #[test]
    fn size_in_bytes_non_zero_for_non_empty() {
        let ef = EliasFano::encode(&[1, 2, 3]);
        assert!(ef.size_in_bytes() > 0);
    }

    #[test]
    #[should_panic(expected = "non-decreasing")]
    fn rejects_non_monotone() {
        EliasFano::encode(&[10, 5, 20]);
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn access_out_of_bounds() {
        let ef = EliasFano::encode(&[1, 2, 3]);
        ef.access(3);
    }

    // ── Bit manipulation ────────────────────────────────────────────

    #[test]
    fn get_set_bits_within_word() {
        let mut words = vec![0u64; 2];
        set_bits(&mut words, 0, 8, 0xAB);
        assert_eq!(get_bits(&words, 0, 8), 0xAB);

        set_bits(&mut words, 16, 12, 0xFFF);
        assert_eq!(get_bits(&words, 16, 12), 0xFFF);
    }

    #[test]
    fn get_set_bits_crossing_boundary() {
        let mut words = vec![0u64; 2];
        set_bits(&mut words, 60, 8, 0xFF);
        assert_eq!(get_bits(&words, 60, 8), 0xFF);
    }

    #[test]
    fn select_in_word_various() {
        assert_eq!(select_in_word(0b1010_1010, 0), 1);
        assert_eq!(select_in_word(0b1010_1010, 1), 3);
        assert_eq!(select_in_word(0b1010_1010, 2), 5);
        assert_eq!(select_in_word(0b1010_1010, 3), 7);
        assert_eq!(select_in_word(1, 0), 0);
        assert_eq!(select_in_word(u64::MAX, 63), 63);
    }

    // ── Property tests ──────────────────────────────────────────────

    fn sorted_values_strategy(max_len: usize, max_val: u64) -> impl Strategy<Value = Vec<u64>> {
        prop::collection::vec(0u64..=max_val, 0..=max_len).prop_map(|mut v| {
            v.sort();
            v
        })
    }

    proptest! {
        #[test]
        fn access_matches_original(values in sorted_values_strategy(200, 10_000)) {
            let ef = EliasFano::encode(&values);
            prop_assert_eq!(ef.len(), values.len());
            for (i, &v) in values.iter().enumerate() {
                prop_assert_eq!(ef.access(i), v, "access({}) mismatch", i);
            }
        }

        #[test]
        fn rank_matches_naive(values in sorted_values_strategy(100, 1_000)) {
            let ef = EliasFano::encode(&values);
            // Test rank at every value and some in between
            let mut test_points: Vec<u64> = values.clone();
            for &v in &values {
                if v > 0 { test_points.push(v - 1); }
                test_points.push(v + 1);
            }
            test_points.push(0);
            test_points.sort();
            test_points.dedup();

            for &q in &test_points {
                let naive_rank = values.iter().filter(|&&v| v <= q).count();
                prop_assert_eq!(
                    ef.rank(q), naive_rank,
                    "rank({}) mismatch: ef={}, naive={}",
                    q, ef.rank(q), naive_rank
                );
            }
        }

        #[test]
        fn next_geq_matches_naive(values in sorted_values_strategy(100, 1_000)) {
            let ef = EliasFano::encode(&values);

            let mut test_points: Vec<u64> = values.clone();
            test_points.push(0);
            if let Some(&max) = values.last() {
                test_points.push(max + 1);
            }
            test_points.sort();
            test_points.dedup();

            for &q in &test_points {
                let naive = values.iter().enumerate()
                    .find(|&(_, v)| *v >= q)
                    .map(|(i, &v)| (i, v));
                let ef_result = ef.next_geq(q);
                prop_assert_eq!(
                    ef_result, naive,
                    "next_geq({}) mismatch: ef={:?}, naive={:?}",
                    q, ef_result, naive
                );
            }
        }

        #[test]
        fn select_equals_access(values in sorted_values_strategy(100, 1_000)) {
            let ef = EliasFano::encode(&values);
            for i in 0..values.len() {
                prop_assert_eq!(ef.select(i), ef.access(i));
            }
        }

        #[test]
        fn space_within_10x_of_optimal(values in sorted_values_strategy(500, 100_000)) {
            if values.len() < 2 { return Ok(()); }
            let ef = EliasFano::encode(&values);
            let actual = ef.size_in_bytes();
            let optimal = ef.optimal_size_in_bytes().max(1);
            // Allow 10x overhead (generous, real ratio is typically < 2x)
            prop_assert!(
                actual <= optimal * 10,
                "space: actual={actual}, optimal={optimal}, ratio={}",
                actual as f64 / optimal as f64
            );
        }
    }

    // ── Memory comparison benchmarks ────────────────────────────────

    fn make_prefix_sums(n: usize, avg_height: u64) -> Vec<u64> {
        let mut sums = Vec::with_capacity(n);
        let mut acc = 0u64;
        for i in 0..n {
            acc += avg_height + (i as u64 % 5); // slight variation
            sums.push(acc);
        }
        sums
    }

    #[test]
    fn memory_comparison_1k() {
        let sums = make_prefix_sums(1_000, 20);
        let ef = EliasFano::encode(&sums);
        let dense = sums.len() * 8;
        let ef_size = ef.size_in_bytes();
        assert!(ef_size < dense, "1K: EF={ef_size}B < dense={dense}B");
    }

    #[test]
    fn memory_comparison_10k() {
        let sums = make_prefix_sums(10_000, 20);
        let ef = EliasFano::encode(&sums);
        let dense = sums.len() * 8;
        let ef_size = ef.size_in_bytes();
        assert!(ef_size < dense, "10K: EF={ef_size}B < dense={dense}B");
    }

    #[test]
    fn memory_comparison_100k() {
        let sums = make_prefix_sums(100_000, 20);
        let ef = EliasFano::encode(&sums);
        let dense = sums.len() * 8;
        let ef_size = ef.size_in_bytes();
        assert!(ef_size < dense, "100K: EF={ef_size}B < dense={dense}B");
    }

    #[test]
    fn memory_comparison_1m() {
        let sums = make_prefix_sums(1_000_000, 20);
        let ef = EliasFano::encode(&sums);
        let dense = sums.len() * 8;
        let ef_size = ef.size_in_bytes();
        assert!(ef_size < dense, "1M: EF={ef_size}B < dense={dense}B");
    }

    #[test]
    fn query_correctness_at_scale() {
        let sums = make_prefix_sums(100_000, 20);
        let ef = EliasFano::encode(&sums);

        // Spot-check access
        assert_eq!(ef.access(0), sums[0]);
        assert_eq!(ef.access(50_000), sums[50_000]);
        assert_eq!(ef.access(99_999), sums[99_999]);

        // Spot-check rank
        let mid_val = sums[50_000];
        let naive_rank = sums.iter().filter(|&&v| v <= mid_val).count();
        assert_eq!(ef.rank(mid_val), naive_rank);

        // Spot-check next_geq
        let target = sums[75_000] - 1;
        let result = ef.next_geq(target);
        assert!(result.is_some());
        let (idx, val) = result.unwrap();
        assert!(val >= target);
        assert_eq!(val, sums[idx]);
    }
}
