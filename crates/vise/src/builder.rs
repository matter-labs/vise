use prometheus_client::{encoding::EncodeMetric, metrics::counter::Counter};

use std::hash::Hash;

use crate::{
    traits::{EncodeLabelSet, GaugeValue, HistogramValue},
    wrappers::{Family, Gauge, Histogram, Info},
    Buckets, Metrics,
};

/// Builder of a single metric or a [`Family`] of metrics. Parameterized by buckets
/// (only applicable to [`Histogram`]s and their families) and labels (only applicable
/// to families).
#[derive(Debug, Clone, Copy)]
pub struct MetricBuilder<B = (), L = ()> {
    buckets: B,
    labels: L,
}

impl Default for MetricBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricBuilder {
    /// Creates a builder with buckets and labels not configured.
    pub const fn new() -> Self {
        Self {
            buckets: (),
            labels: (),
        }
    }
}

impl<L> MetricBuilder<(), L> {
    /// Configures buckets for this builder.
    pub fn with_buckets(self, buckets: impl Into<Buckets>) -> MetricBuilder<Buckets, L> {
        MetricBuilder {
            buckets: buckets.into(),
            labels: self.labels,
        }
    }
}

impl<B> MetricBuilder<B> {
    /// Configures labels for this builder.
    pub fn with_labels<L>(self, labels: L) -> MetricBuilder<B, L> {
        MetricBuilder {
            buckets: self.buckets,
            labels,
        }
    }
}

/// Metric that can be constructed from a [`MetricBuilder`].
pub trait BuildMetric: 'static + Sized /*+ EncodeMetric + TypedMetric*/ {
    /// Metric builder used to construct a metric.
    type Builder: Copy;

    /// Creates a metric given its builder.
    fn build(builder: Self::Builder) -> Self;
}

impl<N, A> BuildMetric for Counter<N, A>
where
    Counter<N, A>: 'static + EncodeMetric + Default,
{
    type Builder = MetricBuilder;

    fn build(_builder: Self::Builder) -> Self {
        Self::default()
    }
}

impl<V: GaugeValue> BuildMetric for Gauge<V> {
    type Builder = MetricBuilder;

    fn build(_builder: Self::Builder) -> Self {
        Self::default()
    }
}

impl<V: HistogramValue> BuildMetric for Histogram<V> {
    type Builder = MetricBuilder<Buckets>;

    fn build(builder: Self::Builder) -> Self {
        Histogram::new(builder.buckets)
    }
}

impl<S: 'static + EncodeLabelSet> BuildMetric for Info<S> {
    type Builder = MetricBuilder;

    fn build(_builder: Self::Builder) -> Self {
        Self::default()
    }
}

impl<S, M, B, L> BuildMetric for Family<S, M, L>
where
    S: 'static + Clone + Eq + Hash,
    M: BuildMetric<Builder = MetricBuilder<B, ()>>,
    B: Copy,
    L: 'static + Copy,
    Family<S, M, L>: EncodeMetric,
{
    type Builder = MetricBuilder<B, L>;

    fn build(builder: Self::Builder) -> Self {
        let item_builder = MetricBuilder {
            buckets: builder.buckets,
            labels: (),
        };
        Family::new(item_builder, builder.labels)
    }
}

impl<M: Metrics + Default> BuildMetric for M {
    type Builder = (); // FIXME: ??

    fn build(_builder: Self::Builder) -> Self {
        Self::default()
    }
}
