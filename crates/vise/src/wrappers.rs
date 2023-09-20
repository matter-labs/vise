//! Wrappers for metric types defined in `prometheus-client`.

use elsa::sync::FrozenMap;
use prometheus_client::{
    encoding::{EncodeLabelSet, EncodeMetric, MetricEncoder},
    metrics::{
        family::MetricConstructor, gauge::Gauge as GaugeInner,
        histogram::Histogram as HistogramInner, MetricType, TypedMetric,
    },
};

use std::{
    collections::HashMap,
    fmt,
    hash::Hash,
    marker::PhantomData,
    ops,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    buckets::Buckets,
    constructor::ConstructMetric,
    traits::{EncodedGaugeValue, GaugeValue, HistogramValue},
};

/// Gauge metric.
///
/// Gauges are integer or floating-point values that can go up or down. Logically, a reported gauge value
/// can be treated as valid until the next value is reported.
///
/// Gauge values must implement the [`GaugeValue`] trait.
pub struct Gauge<V: GaugeValue = i64>(GaugeInner<V, V::Atomic>);

impl<V: GaugeValue> fmt::Debug for Gauge<V> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, formatter)
    }
}

impl<V: GaugeValue> Clone for Gauge<V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<V: GaugeValue> Default for Gauge<V> {
    fn default() -> Self {
        Self(GaugeInner::default())
    }
}

impl<V: GaugeValue> Gauge<V> {
    /// Increases this [`Gauge`] by `v`, returning the previous value.
    pub fn inc_by(&self, v: V) -> V {
        self.0.inc_by(v)
    }

    /// Decreases this [`Gauge`] by `v`, returning the previous value.
    ///
    /// # Panics
    ///
    /// Depending on the value type, this method may panic on underflow; use with care.
    pub fn dec_by(&self, v: V) -> V {
        self.0.dec_by(v)
    }

    /// Sets the value of this [`Gauge`] returning the previous value.
    pub fn set(&self, value: V) -> V {
        self.0.set(value)
    }

    /// Gets the current value of the gauge.
    pub fn get(&self) -> V {
        self.0.get()
    }
}

impl<V: GaugeValue> EncodeMetric for Gauge<V> {
    fn encode(&self, mut encoder: MetricEncoder<'_, '_>) -> fmt::Result {
        match self.get().encode() {
            EncodedGaugeValue::I64(value) => encoder.encode_gauge(&value),
            EncodedGaugeValue::F64(value) => encoder.encode_gauge(&value),
        }
    }

    fn metric_type(&self) -> MetricType {
        <Self as TypedMetric>::TYPE
    }
}

impl<V: GaugeValue> TypedMetric for Gauge<V> {
    const TYPE: MetricType = MetricType::Gauge;
}

/// Histogram metric.
///
/// Histograms are floating-point values counted in configurable buckets. Logically, a histogram observes
/// a certain probability distribution, and observations are transient (unlike gauge values).
///
/// Histogram values must implement the [`HistogramValue`] trait.
#[derive(Debug)]
pub struct Histogram<V: HistogramValue = f64> {
    inner: HistogramInner,
    _value: PhantomData<V>,
}

impl<V: HistogramValue> Clone for Histogram<V> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _value: PhantomData,
        }
    }
}

impl<V: HistogramValue> Histogram<V> {
    pub(crate) fn new(buckets: Buckets) -> Self {
        Self {
            inner: HistogramInner::new(buckets.iter()),
            _value: PhantomData,
        }
    }

    /// Observes the specified `value` of the metric.
    pub fn observe(&self, value: V) {
        self.inner.observe(value.encode());
    }
}

impl Histogram<Duration> {
    /// Starts latency observation for the metric. When the observation is finished,
    /// call [`LatencyObserver::observe()`].
    pub fn start(&self) -> LatencyObserver<'_> {
        LatencyObserver {
            start: Instant::now(),
            histogram: self,
        }
    }
}

impl<V: HistogramValue> EncodeMetric for Histogram<V> {
    fn encode(&self, encoder: MetricEncoder<'_, '_>) -> fmt::Result {
        self.inner.encode(encoder)
    }

    fn metric_type(&self) -> MetricType {
        <Self as TypedMetric>::TYPE
    }
}

impl<V: HistogramValue> TypedMetric for Histogram<V> {
    const TYPE: MetricType = MetricType::Histogram;
}

/// Observer of latency for a [`Histogram`].
#[must_use = "`LatencyObserver` should be `observe()`d"]
#[derive(Debug)]
pub struct LatencyObserver<'a> {
    start: Instant,
    histogram: &'a Histogram<Duration>,
}

impl LatencyObserver<'_> {
    /// Observes and returns the latency passed since this observer was created.
    pub fn observe(self) -> Duration {
        let elapsed = self.start.elapsed();
        self.histogram.observe(elapsed);
        elapsed
    }
}

struct FamilyInner<S, M: ConstructMetric> {
    map: FrozenMap<S, Box<M>>,
    constructor: M::Constructor,
}

impl<S, M> fmt::Debug for FamilyInner<S, M>
where
    S: fmt::Debug + Clone + Eq + Hash,
    M: ConstructMetric + fmt::Debug,
    M::Constructor: fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let map_keys = self.map.keys_cloned();
        let map_snapshot: HashMap<_, _> = map_keys
            .iter()
            .map(|key| (key, self.map.get(key).unwrap()))
            .collect();

        formatter
            .debug_struct("Family")
            .field("map", &map_snapshot)
            .field("constructor", &self.constructor)
            .finish()
    }
}

impl<S, M> FamilyInner<S, M>
where
    S: Clone + Eq + Hash,
    M: ConstructMetric,
{
    fn get_or_create(&self, labels: &S) -> &M {
        if let Some(metric) = self.map.get(labels) {
            return metric;
        }
        self.map
            .insert_with(labels.clone(), || Box::new(self.constructor.new_metric()))
    }
}

/// Family of metrics labelled by one or more labels.
///
/// Family members can be accessed by indexing.
pub struct Family<S, M: ConstructMetric>(Arc<FamilyInner<S, M>>);

impl<S, M> fmt::Debug for Family<S, M>
where
    S: fmt::Debug + Clone + Eq + Hash,
    M: ConstructMetric + fmt::Debug,
    M::Constructor: fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, formatter)
    }
}

impl<S, M: ConstructMetric> Clone for Family<S, M> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<S: Clone + Eq + Hash, M: ConstructMetric> Family<S, M> {
    pub(crate) fn new(constructor: M::Constructor) -> Self {
        let inner = FamilyInner {
            map: FrozenMap::new(),
            constructor,
        };
        Self(Arc::new(inner))
    }

    /// Checks whether this family contains a metric with the specified labels. This is mostly useful
    /// for testing.
    pub fn contains(&self, labels: &S) -> bool {
        self.0.map.get(labels).is_some()
    }

    /// Gets a metric with the specified labels if it was reported previously. This is mostly useful
    /// for testing; use indexing for reporting.
    pub fn get(&self, labels: &S) -> Option<&M> {
        self.0.map.get(labels)
    }

    /// Returns all metrics currently present in this family together with the corresponding labels.
    /// This is inefficient and mostly useful for testing purposes.
    #[allow(clippy::missing_panics_doc)] // false positive
    pub fn to_entries(&self) -> HashMap<S, &M> {
        let labels = self.0.map.keys_cloned();
        labels
            .into_iter()
            .map(|key| {
                let metric = self.0.map.get(&key).unwrap();
                (key, metric)
            })
            .collect()
    }
}

/// Will create a new metric with the specified labels if it's missing in the family.
impl<S: Clone + Eq + Hash, M: ConstructMetric> ops::Index<&S> for Family<S, M> {
    type Output = M;

    fn index(&self, labels: &S) -> &Self::Output {
        self.0.get_or_create(labels)
    }
}

impl<S, M: ConstructMetric> EncodeMetric for Family<S, M>
where
    S: Clone + Eq + Hash + EncodeLabelSet,
{
    fn encode(&self, mut encoder: MetricEncoder<'_, '_>) -> fmt::Result {
        for labels in &self.0.map.keys_cloned() {
            let metric = self.0.map.get(labels).unwrap();
            let encoder = encoder.encode_family(labels)?;
            metric.encode(encoder)?;
        }
        Ok(())
    }

    fn metric_type(&self) -> MetricType {
        <Self as TypedMetric>::TYPE
    }
}

impl<S, M: ConstructMetric> TypedMetric for Family<S, M> {
    const TYPE: MetricType = <M as TypedMetric>::TYPE;
}

#[cfg(test)]
mod tests {
    use prometheus_client::metrics::family::Family as StandardFamily;

    use std::{sync::mpsc, thread};

    use super::*;

    type Label = (&'static str, &'static str);

    #[test]
    fn standard_family_is_easy_to_deadlock() {
        let (stop_sender, stop_receiver) = mpsc::channel();
        thread::spawn(move || {
            let family = StandardFamily::<Label, Histogram<Duration>, _>::new_with_constructor(
                Buckets::LATENCIES,
            );
            let first_metric = family.get_or_create(&("method", "test"));
            let second_metric = family.get_or_create(&("method", "other"));
            // ^ The second call will deadlock because of how `Family` is organized internally; its
            // `get_or_create()` provides a read guard for the internal map, and creating a new metric
            // requires a write lock on the same map.

            first_metric.observe(Duration::from_millis(10));
            second_metric.observe(Duration::from_millis(20));
            stop_sender.send(()).ok();
        });

        let err = stop_receiver
            .recv_timeout(Duration::from_millis(200))
            .unwrap_err();
        assert!(matches!(err, mpsc::RecvTimeoutError::Timeout));
    }

    #[test]
    fn family_accesses_are_not_deadlocked() {
        let family = Family::<Label, Histogram<Duration>>::new(Buckets::LATENCIES);
        let first_metric = &family[&("method", "test")];
        let second_metric = &family[&("method", "other")];
        first_metric.observe(Duration::from_millis(10));
        second_metric.observe(Duration::from_millis(20));

        // We circumvent deadlocking problems by using a *frozen map* (one that can be updated via a shared ref).
        // See its docs for more details. As an added bonus, we can use indexing notation instead of
        // clunky methods!
    }
}
