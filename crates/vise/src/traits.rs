//! Traits used for metric definitions, such as [`GaugeValue`] and [`HistogramValue`].

use prometheus_client::{
    encoding::{EncodeLabel, EncodeLabelSet, EncodeLabelValue, LabelSetEncoder, LabelValueEncoder},
    metrics::gauge,
};

use std::{
    fmt,
    sync::atomic::{AtomicI64, AtomicIsize, AtomicU64, AtomicUsize, Ordering},
    time::Duration,
};

/// Encoded value of a gauge.
#[derive(Debug, Clone, Copy)]
pub enum EncodedGaugeValue {
    /// Signed integer value.
    I64(i64),
    /// Floating point value.
    F64(f64),
}

/// Value of a [`Gauge`](crate::Gauge).
///
/// This trait is implemented for signed and unsigned integers (`i64`, `u64`, `isize`, `usize`),
/// `f64` and [`Duration`]. To use smaller ints and floats as `Gauge` values,
/// they can be converted to their larger-sized variants (e.g., `i16` to `i64`, `u32` to `u64`,
/// and `f32` to `f64`).
pub trait GaugeValue: 'static + Copy + fmt::Debug {
    /// Atomic store for the value.
    type Atomic: gauge::Atomic<Self> + Default + fmt::Debug;
    /// Encodes this value for exporting.
    fn encode(self) -> EncodedGaugeValue;
}

impl GaugeValue for i64 {
    type Atomic = AtomicI64;

    fn encode(self) -> EncodedGaugeValue {
        EncodedGaugeValue::I64(self)
    }
}

impl GaugeValue for u64 {
    type Atomic = AtomicU64Wrapper; // Can't use `AtomicU64` due to orphaning rules

    #[allow(clippy::cast_precision_loss)] // OK for reporting
    fn encode(self) -> EncodedGaugeValue {
        i64::try_from(self).map_or_else(
            |_| EncodedGaugeValue::F64(self as f64),
            EncodedGaugeValue::I64,
        )
    }
}

/// Thin wrapper around [`AtomicU64`] used as atomic store for `u64`.
///
/// A separate type is necessary to circumvent Rust orphaning rules.
#[derive(Debug, Default)]
pub struct AtomicU64Wrapper(AtomicU64);

macro_rules! impl_atomic_wrapper {
    ($wrapper:ty => $int:ty) => {
        impl gauge::Atomic<$int> for $wrapper {
            fn inc(&self) -> $int {
                self.inc_by(1)
            }

            fn inc_by(&self, v: $int) -> $int {
                self.0.fetch_add(v, Ordering::Relaxed)
            }

            fn dec(&self) -> $int {
                self.dec_by(1)
            }

            fn dec_by(&self, v: $int) -> $int {
                self.0.fetch_sub(v, Ordering::Relaxed)
            }

            fn set(&self, v: $int) -> $int {
                self.0.swap(v, Ordering::Relaxed)
            }

            fn get(&self) -> $int {
                self.0.load(Ordering::Relaxed)
            }
        }
    };
}

impl_atomic_wrapper!(AtomicU64Wrapper => u64);

impl GaugeValue for usize {
    type Atomic = AtomicUsizeWrapper; // Can't use `AtomicUsize` due to orphaning rules

    fn encode(self) -> EncodedGaugeValue {
        GaugeValue::encode(self as u64)
    }
}

/// Thin wrapper around [`AtomicUsize`] used as atomic store for `usize`.
///
/// A separate type is necessary to circumvent Rust orphaning rules.
#[derive(Debug, Default)]
pub struct AtomicUsizeWrapper(AtomicUsize);

impl_atomic_wrapper!(AtomicUsizeWrapper => usize);

impl GaugeValue for isize {
    type Atomic = AtomicIsizeWrapper; // Can't use `AtomicIsize` due to orphaning rules

    fn encode(self) -> EncodedGaugeValue {
        EncodedGaugeValue::I64(self as i64)
    }
}

/// Thin wrapper around [`AtomicIsize`] used as atomic store for `isize`.
///
/// A separate type is necessary to circumvent Rust orphaning rules.
#[derive(Debug, Default)]
pub struct AtomicIsizeWrapper(AtomicIsize);

impl_atomic_wrapper!(AtomicIsizeWrapper => isize);

impl GaugeValue for f64 {
    type Atomic = AtomicU64;

    fn encode(self) -> EncodedGaugeValue {
        EncodedGaugeValue::F64(self)
    }
}

impl GaugeValue for Duration {
    type Atomic = AtomicU64Wrapper;

    fn encode(self) -> EncodedGaugeValue {
        EncodedGaugeValue::F64(self.as_secs_f64())
    }
}

impl gauge::Atomic<Duration> for AtomicU64Wrapper {
    fn inc(&self) -> Duration {
        self.inc_by(Duration::from_secs(1))
    }

    fn inc_by(&self, v: Duration) -> Duration {
        Duration::from_secs_f64(self.0.inc_by(v.as_secs_f64()))
    }

    fn dec(&self) -> Duration {
        self.dec_by(Duration::from_secs(1))
    }

    fn dec_by(&self, v: Duration) -> Duration {
        Duration::from_secs_f64(self.0.dec_by(v.as_secs_f64()))
    }

    fn set(&self, v: Duration) -> Duration {
        Duration::from_secs_f64(self.0.set(v.as_secs_f64()))
    }

    fn get(&self) -> Duration {
        Duration::from_secs_f64(self.0.get())
    }
}

/// Value of a [`Histogram`](crate::Histogram).
///
/// This trait is implemented for signed and unsigned integers (`i64`, `u64`, `isize`, `usize`),
/// `f64` and [`Duration`]. To use smaller ints and floats as `Histogram` values,
/// they can be converted to their larger-sized variants (e.g., `i16` to `i64`, `u32` to `u64`,
/// and `f32` to `f64`).
pub trait HistogramValue: 'static + Copy + fmt::Debug {
    /// Encodes this value for exporting.
    fn encode(self) -> f64;
}

impl HistogramValue for f64 {
    fn encode(self) -> f64 {
        self
    }
}

impl HistogramValue for Duration {
    fn encode(self) -> f64 {
        self.as_secs_f64()
    }
}

macro_rules! impl_histogram_value_for_int {
    ($int:ty) => {
        impl HistogramValue for $int {
            fn encode(self) -> f64 {
                self as f64
            }
        }
    };
}

impl_histogram_value_for_int!(i64);
impl_histogram_value_for_int!(u64);
impl_histogram_value_for_int!(usize);
impl_histogram_value_for_int!(isize);

/// Maps a set of labels from the storage format (i.e., how labels are stored in a [`Family`](crate::Family))
/// to the encoding format, which is used when [exporting metrics](crate::Registry::encode()).
pub trait MapLabels<S>: Copy {
    /// Result of the mapping.
    type Output<'a>: EncodeLabelSet
    where
        Self: 'a,
        S: 'a;
    /// Performs mapping.
    fn map_labels<'a>(&'a self, labels: &'a S) -> Self::Output<'a>;
}

/// Identity mapping.
impl<S: EncodeLabelSet> MapLabels<S> for () {
    type Output<'a> = LabelRef<'a, S> where S: 'a;

    fn map_labels<'a>(&'a self, labels: &'a S) -> Self::Output<'a> {
        LabelRef(labels)
    }
}

/// Wrapper around a reference to labels proxying necessary trait implementations.
// We cannot use a reference directly because there are no blanket implementations for `EncodeLabelSet`
// and `EncodeLabelValue`.
#[derive(Debug)]
pub struct LabelRef<'a, S>(pub &'a S);

impl<S: EncodeLabelSet> EncodeLabelSet for LabelRef<'_, S> {
    fn encode(&self, encoder: LabelSetEncoder) -> fmt::Result {
        self.0.encode(encoder)
    }
}

impl<S: EncodeLabelValue> EncodeLabelValue for LabelRef<'_, S> {
    fn encode(&self, encoder: &mut LabelValueEncoder) -> fmt::Result {
        self.0.encode(encoder)
    }
}

/// Set of metric labels with label names known during compilation. Used as output in [`MapLabels`]
/// implementations.
#[derive(Debug)]
pub struct StaticLabelSet<'a, S: 'a> {
    label_keys: &'a [&'static str],
    label_values: S,
}

impl<S: EncodeLabelValue> MapLabels<S> for [&'static str; 1] {
    type Output<'a> = StaticLabelSet<'a, (&'a S,)> where S: 'a;

    fn map_labels<'a>(&'a self, labels: &'a S) -> Self::Output<'a> {
        StaticLabelSet {
            label_keys: self,
            label_values: (labels,),
        }
    }
}

macro_rules! impl_map_labels {
    ($len:tt => $($idx:tt : $typ:ident),+) => {
        impl<$($typ,)+> MapLabels<($($typ,)+)> for [&'static str; $len]
        where
            $($typ: EncodeLabelValue,)+
        {
            type Output<'a> = StaticLabelSet<'a, ($(&'a $typ,)+)> where $($typ: 'a,)+;

            fn map_labels<'a>(&'a self, labels: &'a ($($typ,)+)) -> Self::Output<'a> {
                StaticLabelSet {
                    label_keys: self,
                    label_values: ($(&labels.$idx,)+),
                }
            }
        }
    };
}

impl_map_labels!(2 => 0: S0, 1: S1);
impl_map_labels!(3 => 0: S0, 1: S1, 2: S2);
impl_map_labels!(4 => 0: S0, 1: S1, 2: S2, 3: S3);

macro_rules! impl_encode_for_static_label_set {
    ($len:tt => $($idx:tt : $typ:ident),+) => {
        impl<'a, $($typ,)+> EncodeLabelSet for StaticLabelSet<'a, ($(&'a $typ,)+)>
        where
            $($typ: EncodeLabelValue,)+
        {
            fn encode(&self, mut encoder: LabelSetEncoder) -> fmt::Result {
                $(
                let label = (self.label_keys[$idx], LabelRef(self.label_values.$idx));
                EncodeLabel::encode(&label, encoder.encode_label())?;
                )+
                Ok(())
            }
        }
    };
}

impl_encode_for_static_label_set!(1 => 0: S);
impl_encode_for_static_label_set!(2 => 0: S0, 1: S1);
impl_encode_for_static_label_set!(3 => 0: S0, 1: S1, 2: S2);
impl_encode_for_static_label_set!(4 => 0: S0, 1: S1, 2: S2, 3: S3);
