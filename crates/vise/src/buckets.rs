use std::{cmp, iter, mem, ops};

use compile_fmt::{compile_assert, fmt};

#[derive(Debug, Clone, Copy)]
enum BucketsInner {
    Slice(&'static [f64]),
    Linear {
        start: f64,
        end: f64,
        step: f64,
    },
    Exponential {
        start: f64,
        end: f64,
        factor: f64,
    },
    Scaled {
        start: f64,
        end: f64,
        factors: &'static [f64],
    },
}

impl BucketsInner {
    const fn smallest_value(&self) -> f64 {
        match self {
            Self::Slice(values) => values[0],
            Self::Linear { start, .. }
            | Self::Exponential { start, .. }
            | Self::Scaled { start, .. } => *start,
        }
    }

    fn iter(self) -> Box<dyn Iterator<Item = f64>> {
        match self {
            Self::Slice(slice) => Box::new(slice.iter().copied()),
            Self::Linear { start, end, step } => {
                let it = iter::successors(Some(start), move |&value| {
                    let value = value + step;
                    (value <= end).then_some(value)
                });
                Box::new(it)
            }
            Self::Exponential { start, end, factor } => {
                let it = iter::successors(Some(start), move |&value| {
                    let value = value * factor;
                    (value <= end).then_some(value)
                });
                Box::new(it)
            }
            Self::Scaled {
                start,
                end,
                factors,
            } => {
                let greatest_factor = *factors.last().unwrap();
                let smaller_factors = &factors[..factors.len() - 1];

                let starts =
                    iter::successors(Some(start), move |&value| Some(value * greatest_factor));

                let it = starts
                    .flat_map(move |start| {
                        iter::once(1.0)
                            .chain(smaller_factors.iter().copied())
                            .map(move |factor| start * factor)
                    })
                    .take_while(move |&value| value <= end);
                Box::new(it)
            }
        }
    }
}

/// Buckets configuration for a [`Histogram`](crate::Histogram) or a [`Family`](crate::Family) of histograms.
#[derive(Debug, Clone, Copy)]
pub struct Buckets {
    inner: BucketsInner,
    bias: f64,
    mirrored: bool,
}

impl Buckets {
    /// Default buckets configuration for latencies.
    pub const LATENCIES: Self =
        Self::values(&[0.001, 0.005, 0.025, 0.1, 0.25, 1.0, 5.0, 30.0, 120.0]);

    /// Linear buckets covering `[0.0, 1.0]` interval.
    pub const ZERO_TO_ONE: Self = Self::values(&[0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9]);

    const fn new(inner: BucketsInner) -> Self {
        Self {
            inner,
            bias: 0.0,
            mirrored: false,
        }
    }

    /// Creates buckets based on the provided `values`.
    ///
    /// # Panics
    ///
    /// Panics if `values` are empty or are not monotonically increasing.
    #[track_caller]
    pub const fn values(values: &'static [f64]) -> Self {
        assert!(!values.is_empty(), "Values cannot be empty");

        let mut i = 1;
        let mut prev_value = values[0];
        while i < values.len() {
            compile_assert!(
                is_f64_greater(values[i], prev_value),
                "Values must be monotonically increasing; offending value has index ",
                i => fmt::<usize>()
            );
            prev_value = values[i];
            i += 1;
        }
        Self::new(BucketsInner::Slice(values))
    }

    /// Creates linear buckets based on the specified `range` and `step`. The created buckets will
    /// consist of `range.start`, `range.start + step`, ..., until `range.end` (potentially inclusive).
    ///
    /// # Panics
    ///
    /// Panics if `range` is empty, or if `step` is not positive.
    pub const fn linear(range: ops::RangeInclusive<f64>, step: f64) -> Self {
        assert!(
            is_f64_greater(*range.end(), *range.start()),
            "Specified linear range is empty"
        );
        assert!(is_f64_greater(step, 0.0), "Step must be positive");
        Self::new(BucketsInner::Linear {
            start: *range.start(),
            end: *range.end(),
            step,
        })
    }

    /// Creates exponential buckets based on the specified `range` and `factor`. The created buckets
    /// will consist of `range.start`, `range.start * factor`, ... until `range.end` (potentially inclusive).
    ///
    /// # Panics
    ///
    /// Panics if `range` is empty, `range.start <= 0` or `factor <= 1`.
    pub const fn exponential(range: ops::RangeInclusive<f64>, factor: f64) -> Self {
        assert!(
            is_f64_greater(*range.start(), 0.0),
            "Range start must be positive"
        );
        assert!(
            is_f64_greater(*range.end(), *range.start()),
            "Specified exponential range is empty"
        );
        assert!(is_f64_greater(factor, 1.0), "Factor must be greater than 1");
        Self::new(BucketsInner::Exponential {
            start: *range.start(),
            end: *range.end(),
            factor,
        })
    }

    /// Creates *roughly* exponential buckets that apply the given sequence of `factors` to the `range`.
    /// `factors` must be monotonically increasing and exceed 1.
    ///
    /// The created buckets will consist of:
    ///
    /// - `range.start` multiplied by 1.0, `factors[0]`, `factors[1]`, ..., `factors[n - 2]`, where
    ///   `n == factors.len()`.
    /// - `range.start * factors[n - 1]` multiplied by 1.0, `factors[0]`, `factors[1]`, ..., `factors[n - 2]`
    /// - ...and so on, until the produced value exceeds `range.end`.
    ///
    /// [Exponential buckets](Self::exponential()) are equivalent to specifying `factors` with a single item.
    ///
    /// # Panics
    ///
    /// Panics if `range` is empty, `factors` are below 1 or not monotonically increasing.
    ///
    /// # Examples
    ///
    /// ```
    /// # use vise::Buckets;
    ///
    /// const BUCKETS: Buckets = Buckets::scaled(1.0..=1_000.0, &[2.0, 5.0, 10.0]);
    /// // `BUCKETS` consist of [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000].
    /// ```
    pub const fn scaled(range: ops::RangeInclusive<f64>, factors: &'static [f64]) -> Self {
        assert!(
            is_f64_greater(*range.start(), 0.0),
            "Range start must be positive"
        );
        assert!(
            is_f64_greater(*range.end(), *range.start()),
            "Specified exponential range is empty"
        );

        assert!(!factors.is_empty(), "At least one factor must be specified");
        assert!(
            is_f64_greater(factors[0], 1.0),
            "Factors must be greater than 1"
        );

        let mut i = 0;
        while i + 1 < factors.len() {
            compile_assert!(
                is_f64_greater(factors[i + 1], factors[i]),
                "Factors must be monotonically increasing; offending value has index ",
                i => fmt::<usize>()
            );
            i += 1;
        }

        Self::new(BucketsInner::Scaled {
            start: *range.start(),
            end: *range.end(),
            factors,
        })
    }

    /// Mirrors the buckets around zero, adding negative values that correspond to all positive
    /// bucket thresholds.
    ///
    /// For example, if the buckets are `[1, 2, 5]`, then after mirroring, they will be
    /// `[-5, -2, -1, 1, 2, 5]`.
    ///
    /// If [a bias](Self::biased()) is set up, it is applied after mirroring.
    ///
    /// # Panics
    ///
    /// Panics if the smallest bucket value is negative.
    #[must_use]
    pub const fn mirrored(self) -> Self {
        assert!(
            is_f64_geq(self.inner.smallest_value(), 0.0),
            "Smallest bucket value must be non-negative"
        );
        Self {
            mirrored: true,
            ..self
        }
    }

    /// Specifies bias for these buckets. This allows to more easily reuse buckets, or to specify
    /// exponential / scaled buckets with bias.
    ///
    /// If called multiple times, the bias is *replaced*, not accumulated. Bias is applied
    /// after [mirroring](Self::mirrored()).
    #[must_use]
    pub const fn biased(self, bias: f64) -> Self {
        Self { bias, ..self }
    }

    pub(crate) fn iter(self) -> impl Iterator<Item = f64> {
        let mut base = self.inner.iter();
        if self.mirrored {
            let collected_base: Vec<_> = self.inner.iter().collect();
            let mirrored = collected_base
                .into_iter()
                .rev()
                .filter_map(|value| (value > 0.0).then_some(-value));
            base = Box::new(mirrored.chain(base));
        }
        base.map(move |value| value + self.bias)
    }
}

impl<const N: usize> From<&'static [f64; N]> for Buckets {
    fn from(values: &'static [f64; N]) -> Self {
        Self::values(values)
    }
}

const fn compare_u64(lhs: u64, rhs: u64) -> cmp::Ordering {
    if lhs < rhs {
        cmp::Ordering::Less
    } else if lhs > rhs {
        cmp::Ordering::Greater
    } else {
        cmp::Ordering::Equal
    }
}

/// Compares `f64` values in compilation time.
const fn compare_f64(lhs: f64, rhs: f64) -> Option<cmp::Ordering> {
    // Since the endianness is the same for `f64` and `u64`, we can use fixed masks regardless of it.
    const FRACTION_MASK: u64 = (1 << 52) - 1;
    const EXPONENT_MASK: u64 = 0x7ff << 52;
    const SIGN_MASK: u64 = 1 << 63;

    #[derive(Debug)]
    struct DecomposedF64 {
        sign_bit: u64,
        exponent_bits: u64,
        fraction_bits: u64,
    }

    impl DecomposedF64 {
        const fn new(bits: u64) -> Self {
            Self {
                sign_bit: bits & SIGN_MASK,
                exponent_bits: bits & EXPONENT_MASK,
                fraction_bits: bits & FRACTION_MASK,
            }
        }

        const fn is_zero(&self) -> bool {
            self.exponent_bits == 0 && self.fraction_bits == 0
        }

        const fn is_subnormal(&self) -> bool {
            self.exponent_bits == 0 && self.fraction_bits != 0
        }

        const fn is_nan(&self) -> bool {
            self.exponent_bits == EXPONENT_MASK && self.fraction_bits != 0
        }
    }

    // SAFETY: transmuting `f64` on its own is safe (it's plain old bits); what is problematic
    // and is the cause of `f64::{to_bits, from_bits}` being non-const is handling corner cases
    // (e.g., NaNs and subnormals) in a platform-independent way consistent with runtime behavior.
    // We check for these corner case numbers below and treat them as non-comparable.
    #[expect(unnecessary_transmutes)]
    // ^ false positive; `f64::to_bits` is stabilized as const fn in Rust 1.83 (i.e., > MSRV).
    let lhs_bits: u64 = unsafe { mem::transmute(lhs) };
    let lhs = DecomposedF64::new(lhs_bits);
    #[expect(unnecessary_transmutes)]
    let rhs_bits: u64 = unsafe { mem::transmute(rhs) };
    let rhs = DecomposedF64::new(rhs_bits);

    if lhs.is_nan() || rhs.is_nan() || lhs.is_subnormal() || rhs.is_subnormal() {
        return None;
    }
    if lhs.is_zero() && rhs.is_zero() {
        return Some(cmp::Ordering::Equal);
    }

    let sign_ordering = compare_u64(lhs.sign_bit, rhs.sign_bit).reverse();
    if !sign_ordering.is_eq() {
        return Some(sign_ordering);
    }

    let mut exponent_ordering = compare_u64(lhs.exponent_bits, rhs.exponent_bits);
    if lhs.sign_bit != 0 {
        // Values are negative; the ordering must be reversed.
        exponent_ordering = exponent_ordering.reverse();
    }
    if !exponent_ordering.is_eq() {
        return Some(exponent_ordering);
    }

    let mut fraction_ordering = compare_u64(lhs.fraction_bits, rhs.fraction_bits);
    if lhs.sign_bit != 0 {
        fraction_ordering = fraction_ordering.reverse();
    }
    Some(fraction_ordering)
}

const fn is_f64_greater(lhs: f64, rhs: f64) -> bool {
    matches!(compare_f64(lhs, rhs), Some(cmp::Ordering::Greater))
}

const fn is_f64_geq(lhs: f64, rhs: f64) -> bool {
    matches!(
        compare_f64(lhs, rhs),
        Some(cmp::Ordering::Greater | cmp::Ordering::Equal)
    )
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // We *want* exact comparisons
mod tests {
    use rand::{rngs::StdRng, Rng, SeedableRng};

    use super::*;

    #[test]
    fn linear_buckets() {
        let buckets = Buckets::linear(0.0..=10.0, 1.0);
        let buckets = buckets.iter().collect::<Vec<_>>();
        assert_eq!(
            buckets,
            [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]
        );
    }

    #[test]
    fn exponential_buckets() {
        let buckets = Buckets::exponential(1.0..=10.0, 2.0);
        let buckets = buckets.iter().collect::<Vec<_>>();
        assert_eq!(buckets, [1.0, 2.0, 4.0, 8.0]);
    }

    #[test]
    fn scaled_buckets() {
        let buckets = Buckets::scaled(1.0..=100.0, &[2.0, 5.0, 10.0]);
        let buckets = buckets.iter().collect::<Vec<_>>();
        assert_eq!(buckets, [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0]);

        let buckets = Buckets::scaled(1.0..=200.0, &[3.0, 10.0]);
        let buckets = buckets.iter().collect::<Vec<_>>();
        assert_eq!(buckets, [1.0, 3.0, 10.0, 30.0, 100.0]);
    }

    #[test]
    #[should_panic(expected = "Range start must be positive")]
    fn incorrect_start_for_scaled_buckets() {
        Buckets::scaled(-1.0..=100.0, &[2.0, 5.0, 10.0]);
    }

    #[test]
    #[should_panic(expected = "exponential range is empty")]
    fn incorrect_end_for_scaled_buckets() {
        Buckets::scaled(1.0..=0.1, &[2.0, 5.0, 10.0]);
    }

    #[test]
    #[should_panic(expected = "Factors must be greater than 1")]
    fn incorrect_start_factor_for_scaled_buckets() {
        Buckets::scaled(1.0..=100.0, &[1.0, 5.0, 10.0]);
    }

    #[test]
    #[should_panic(
        expected = "Factors must be monotonically increasing; offending value has index 0"
    )]
    fn incorrect_factors_sequence_for_scaled_buckets() {
        Buckets::scaled(1.0..=100.0, &[5.0, 2.0, 10.0]);
    }

    #[test]
    fn biased_exponential_buckets() {
        let buckets = Buckets::exponential(1.0..=10.0, 2.0).biased(10.0);
        let buckets = buckets.iter().collect::<Vec<_>>();
        assert_eq!(buckets, [11.0, 12.0, 14.0, 18.0]);

        let buckets = Buckets::scaled(1.0..=200.0, &[3.0, 10.0]).biased(-10.0);
        let buckets = buckets.iter().collect::<Vec<_>>();
        assert_eq!(buckets, [-9.0, -7.0, 0.0, 20.0, 90.0]);
    }

    #[test]
    fn mirrored_buckets() {
        let buckets = Buckets::values(&[0.0, 1.0, 2.0, 5.0]).mirrored();
        let buckets = buckets.iter().collect::<Vec<_>>();
        assert_eq!(buckets, [-5.0, -2.0, -1.0, 0.0, 1.0, 2.0, 5.0]);

        let buckets = Buckets::exponential(1.0..=10.0, 2.0).mirrored();
        let buckets = buckets.iter().collect::<Vec<_>>();
        assert_eq!(buckets, [-8.0, -4.0, -2.0, -1.0, 1.0, 2.0, 4.0, 8.0]);

        let buckets = Buckets::exponential(1.0..=10.0, 2.0).mirrored().biased(8.0);
        let buckets = buckets.iter().collect::<Vec<_>>();
        assert_eq!(buckets, [0.0, 4.0, 6.0, 7.0, 9.0, 10.0, 12.0, 16.0]);

        let buckets = Buckets::scaled(1.0..=200.0, &[3.0, 10.0]).mirrored();
        let buckets = buckets.iter().collect::<Vec<_>>();
        assert_eq!(
            buckets,
            [-100.0, -30.0, -10.0, -3.0, -1.0, 1.0, 3.0, 10.0, 30.0, 100.0]
        );
    }

    #[test]
    fn compare_f64_corner_cases() {
        assert_eq!(compare_f64(0.0, 0.0), Some(cmp::Ordering::Equal));
        assert_eq!(compare_f64(0.0, -0.0), Some(cmp::Ordering::Equal));
        assert_eq!(compare_f64(-0.0, -0.0), Some(cmp::Ordering::Equal));

        assert_eq!(compare_f64(0.0, f64::NAN), None);
        assert_eq!(compare_f64(1.0, f64::NAN), None);
        assert_eq!(compare_f64(f64::INFINITY, f64::NAN), None);
        assert_eq!(compare_f64(-f64::INFINITY, f64::NAN), None);

        assert_eq!(
            compare_f64(f64::INFINITY, 0.0),
            Some(cmp::Ordering::Greater)
        );
        assert_eq!(compare_f64(-f64::INFINITY, 0.0), Some(cmp::Ordering::Less));
        assert_eq!(
            compare_f64(f64::INFINITY, -0.0),
            Some(cmp::Ordering::Greater)
        );
        assert_eq!(compare_f64(-f64::INFINITY, -0.0), Some(cmp::Ordering::Less));

        assert_eq!(
            compare_f64(f64::INFINITY, -f64::INFINITY),
            Some(cmp::Ordering::Greater)
        );
        assert_eq!(
            compare_f64(-f64::INFINITY, f64::INFINITY),
            Some(cmp::Ordering::Less)
        );

        assert_eq!(f64::INFINITY, f64::INFINITY);
        assert_eq!(
            compare_f64(f64::INFINITY, f64::INFINITY),
            Some(cmp::Ordering::Equal)
        );
        assert_eq!(-f64::INFINITY, -f64::INFINITY);
        assert_eq!(
            compare_f64(-f64::INFINITY, -f64::INFINITY),
            Some(cmp::Ordering::Equal)
        );
    }

    #[test]
    fn compare_f64_mini_fuzz() {
        const SEED: u64 = 123;

        let mut rng = StdRng::seed_from_u64(SEED);
        for _ in 0..100_000 {
            let lhs: f64 = rng.random();
            let rhs: f64 = rng.random();
            if lhs.is_subnormal() || rhs.is_subnormal() {
                continue;
            }

            assert_eq!(
                lhs.partial_cmp(&rhs),
                compare_f64(lhs, rhs),
                "Mismatch when comparing {lhs} and {rhs}"
            );
        }

        for _ in 0..100_000 {
            let lhs: f64 = rng.random_range(-1.0..=1.0);
            let rhs: f64 = rng.random_range(-1.0..=1.0);

            assert_eq!(
                lhs.partial_cmp(&rhs),
                compare_f64(lhs, rhs),
                "Mismatch when comparing {lhs} and {rhs}"
            );
        }
    }
}
