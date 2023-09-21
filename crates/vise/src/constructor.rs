use prometheus_client::{
    encoding::EncodeMetric,
    metrics::{counter::Counter, family::MetricConstructor, TypedMetric},
};

use std::hash::Hash;

use crate::{
    traits::{GaugeValue, HistogramValue, MapLabels},
    wrappers::{Family, Gauge, Histogram},
    Buckets, LabeledFamily,
};

/// Metric that can be constructed from a constructor.
///
/// Essentially, this is a dual trait to [`MetricConstructor`] allowing to define a "preferred"
/// constructor for the metric.
pub trait ConstructMetric: 'static + Sized + EncodeMetric + TypedMetric {
    /// Metric constructor.
    type Constructor: MetricConstructor<Self> + Copy;

    /// Creates a metric given its constructor.
    fn construct(constructor: &Self::Constructor) -> Self {
        constructor.new_metric()
    }
}

/// Constructor of metrics implementing the [`Default`] trait (counters, gauges and their families).
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultConstructor;

impl<M: EncodeMetric + Default> MetricConstructor<M> for DefaultConstructor {
    fn new_metric(&self) -> M {
        M::default()
    }
}

impl<N, A> ConstructMetric for Counter<N, A>
where
    Counter<N, A>: 'static + EncodeMetric + Default,
{
    type Constructor = DefaultConstructor;
}

impl<V: GaugeValue> ConstructMetric for Gauge<V> {
    type Constructor = DefaultConstructor;
}

impl<V: HistogramValue> ConstructMetric for Histogram<V> {
    type Constructor = Buckets;
}

impl<V: HistogramValue> MetricConstructor<Histogram<V>> for Buckets {
    fn new_metric(&self) -> Histogram<V> {
        Histogram::new(*self)
    }
}

impl<S, M, C> ConstructMetric for Family<S, M>
where
    S: 'static,
    C: MetricConstructor<Family<S, M>> + Copy,
    M: ConstructMetric<Constructor = C>,
    Family<S, M>: EncodeMetric,
{
    type Constructor = C;
}

impl<const N: usize, S, M, C> ConstructMetric for LabeledFamily<S, M, N>
where
    S: 'static,
    (C, [&'static str; N]): MetricConstructor<LabeledFamily<S, M, N>> + Copy,
    M: ConstructMetric<Constructor = C>,
    LabeledFamily<S, M, N>: EncodeMetric,
{
    type Constructor = (C, [&'static str; N]);
}

impl<S, M> MetricConstructor<Family<S, M>> for DefaultConstructor
where
    S: Clone + Eq + Hash,
    M: ConstructMetric<Constructor = DefaultConstructor>,
{
    fn new_metric(&self) -> Family<S, M> {
        Family::new(DefaultConstructor, ())
    }
}

impl<S, M, L> MetricConstructor<Family<S, M, L>> for (DefaultConstructor, L)
where
    S: Clone + Eq + Hash,
    M: ConstructMetric<Constructor = DefaultConstructor>,
    L: MapLabels<S>,
{
    fn new_metric(&self) -> Family<S, M, L> {
        Family::new(DefaultConstructor, self.1)
    }
}

impl<S, M> MetricConstructor<Family<S, M>> for Buckets
where
    S: Clone + Eq + Hash,
    M: ConstructMetric<Constructor = Buckets>,
{
    fn new_metric(&self) -> Family<S, M> {
        Family::new(*self, ())
    }
}

impl<S, M, L> MetricConstructor<Family<S, M, L>> for (Buckets, L)
where
    S: Clone + Eq + Hash,
    M: ConstructMetric<Constructor = Buckets>,
    L: MapLabels<S>,
{
    fn new_metric(&self) -> Family<S, M, L> {
        Family::new(self.0, self.1)
    }
}
