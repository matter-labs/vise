use prometheus_client::{
    encoding::EncodeMetric,
    metrics::{counter::Counter, family::MetricConstructor, TypedMetric},
};

use std::hash::Hash;

use crate::{
    wrappers::{Family, Gauge, GaugeValue, Histogram, HistogramValue},
    Buckets,
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

impl<S, M> MetricConstructor<Family<S, M>> for DefaultConstructor
where
    S: Clone + Eq + Hash,
    M: ConstructMetric<Constructor = DefaultConstructor>,
{
    fn new_metric(&self) -> Family<S, M> {
        Family::new(DefaultConstructor)
    }
}

impl<S, M> MetricConstructor<Family<S, M>> for Buckets
where
    S: Clone + Eq + Hash,
    M: ConstructMetric<Constructor = Buckets>,
{
    fn new_metric(&self) -> Family<S, M> {
        Family::new(*self)
    }
}
