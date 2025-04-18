//! Wrappers for metric types defined in `prometheus-client`.

use std::{
    collections::HashMap,
    fmt,
    hash::Hash,
    marker::PhantomData,
    ops,
    sync::Arc,
    time::{Duration, Instant},
};

use elsa::sync::FrozenMap;
use once_cell::sync::OnceCell;
use prometheus_client::{
    encoding::{
        EncodeLabelKey, EncodeLabelValue, EncodeMetric, LabelKeyEncoder, LabelValueEncoder,
        MetricEncoder,
    },
    metrics::{
        counter::Counter, gauge::Gauge as GaugeInner, histogram::Histogram as HistogramInner,
        MetricType, TypedMetric,
    },
    registry::Unit,
};

use crate::{
    buckets::Buckets,
    builder::BuildMetric,
    encoding::{EncodeGroupedMetric, FullLabelSet, LabelSetWrapper},
    traits::{EncodeLabelSet, EncodedGaugeValue, GaugeValue, HistogramValue, MapLabels},
};

/// Label with a unit suffix implementing [`EncodeLabelKey`].
#[doc(hidden)] // used in proc macros only
#[derive(Debug)]
pub struct LabelWithUnit {
    name: &'static str,
    unit: Unit,
}

impl LabelWithUnit {
    pub const fn new(name: &'static str, unit: Unit) -> Self {
        Self { name, unit }
    }
}

impl EncodeLabelKey for LabelWithUnit {
    fn encode(&self, encoder: &mut LabelKeyEncoder<'_>) -> fmt::Result {
        use std::fmt::Write as _;

        write!(encoder, "{}_{}", self.name, self.unit.as_str())
    }
}

/// Wraps a [`Duration`] so that it can be used as a label value, which will be set to the fractional
/// number of seconds in the duration, i.e. [`Duration::as_secs_f64()`]. Mostly useful for [`Info`] metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DurationAsSecs(pub Duration);

impl From<Duration> for DurationAsSecs {
    fn from(duration: Duration) -> Self {
        Self(duration)
    }
}

impl EncodeLabelValue for DurationAsSecs {
    fn encode(&self, encoder: &mut LabelValueEncoder) -> fmt::Result {
        EncodeLabelValue::encode(&self.0.as_secs_f64(), encoder)
    }
}

impl<N, A> EncodeGroupedMetric for Counter<N, A> where Self: EncodeMetric + TypedMetric {}

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

    /// Increases this [`Gauge`] by `v` and returns a guard that will decrement this value back
    /// when dropped. This can be useful for gauges that measure consumption of a certain resource.
    pub fn inc_guard(&self, v: V) -> GaugeGuard<V> {
        let guard = GaugeGuard {
            gauge: self.clone(),
            increment: v,
        };
        self.0.inc_by(v);
        guard
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
    fn encode(&self, mut encoder: MetricEncoder<'_>) -> fmt::Result {
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

impl<V: GaugeValue> EncodeGroupedMetric for Gauge<V> {}

/// Guard for a [`Gauge`] returned by [`Gauge::inc_guard()`]. When dropped, a guard decrements
/// the gauge by the same value that it was increased by when creating the guard.
#[derive(Debug)]
pub struct GaugeGuard<V: GaugeValue = i64> {
    gauge: Gauge<V>,
    increment: V,
}

impl<V: GaugeValue> Drop for GaugeGuard<V> {
    fn drop(&mut self) {
        self.gauge.dec_by(self.increment);
    }
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
    fn encode(&self, encoder: MetricEncoder<'_>) -> fmt::Result {
        self.inner.encode(encoder)
    }

    fn metric_type(&self) -> MetricType {
        <Self as TypedMetric>::TYPE
    }
}

impl<V: HistogramValue> TypedMetric for Histogram<V> {
    const TYPE: MetricType = MetricType::Histogram;
}

impl<V: HistogramValue> EncodeGroupedMetric for Histogram<V> {}

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

/// Information metric.
///
/// Information metrics represent pieces of information that are not changed during program lifetime
/// (e.g., config parameters of a certain component).
#[derive(Debug)]
pub struct Info<S>(Arc<OnceCell<S>>);

impl<S> Default for Info<S> {
    fn default() -> Self {
        Self(Arc::default())
    }
}

impl<S> Clone for Info<S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S: EncodeLabelSet> Info<S> {
    /// Gets the current value of the metric.
    pub fn get(&self) -> Option<&S> {
        self.0.get()
    }

    /// Sets the value of this metric.
    ///
    /// # Errors
    ///
    /// Returns an error if the value is already set.
    pub fn set(&self, value: S) -> Result<(), SetInfoError<S>> {
        self.0.set(value).map_err(SetInfoError)
    }
}

impl<S: EncodeLabelSet> EncodeMetric for Info<S> {
    fn encode(&self, mut encoder: MetricEncoder<'_>) -> fmt::Result {
        if let Some(value) = self.0.get() {
            encoder.encode_info(&LabelSetWrapper(value))
        } else {
            Ok(())
        }
    }

    fn metric_type(&self) -> MetricType {
        MetricType::Info
    }
}

impl<S: EncodeLabelSet> TypedMetric for Info<S> {
    const TYPE: MetricType = MetricType::Info;
}

impl<S: EncodeLabelSet> EncodeGroupedMetric for Info<S> {}

/// Error returned from [`Info::set()`].
#[derive(Debug)]
pub struct SetInfoError<S>(S);

impl<S> SetInfoError<S> {
    /// Converts the error into the unsuccessfully set value.
    pub fn into_inner(self) -> S {
        self.0
    }
}

impl<S> fmt::Display for SetInfoError<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("cannot set info metric value; it is already set")
    }
}

pub(crate) struct FamilyInner<S, M: BuildMetric> {
    map: FrozenMap<S, Box<M>>,
    builder: M::Builder,
}

impl<S, M> fmt::Debug for FamilyInner<S, M>
where
    S: fmt::Debug + Clone + Eq + Hash,
    M: BuildMetric + fmt::Debug,
    M::Builder: fmt::Debug,
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
            .field("builder", &self.builder)
            .finish()
    }
}

impl<S, M> FamilyInner<S, M>
where
    S: Clone + Eq + Hash,
    M: BuildMetric,
{
    pub(crate) fn new(builder: M::Builder) -> Self {
        Self {
            map: FrozenMap::new(),
            builder,
        }
    }

    pub(crate) fn get_or_create(&self, labels: &S) -> &M {
        if let Some(metric) = self.map.get(labels) {
            return metric;
        }
        self.map
            .insert_with(labels.clone(), || Box::new(M::build(self.builder)))
    }

    pub(crate) fn to_entries(&self) -> impl ExactSizeIterator<Item = (S, &M)> + '_ {
        let labels = self.map.keys_cloned();
        labels.into_iter().map(|key| {
            let metric = self.map.get(&key).unwrap();
            (key, metric)
        })
    }
}

/// Family of metrics labelled by one or more labels.
///
/// Family members can be accessed by indexing.
pub struct Family<S, M: BuildMetric, L = ()> {
    inner: Arc<FamilyInner<S, M>>,
    labels: L,
}

/// [`Family`] with separately specified label names.
///
/// Separately specifying labels allows to not define newtype wrappers for labels. Instead, labels
/// (the first type param of `LabeledFamily`) can be specified as values (e.g., `&'static str`
/// or `u8`), and the label names are provided separately using the `labels = [..]` attribute
/// with the [`Metrics`](macro@crate::Metrics) derive macro.
///
/// - If there's a single label, its value type must be specified directly: `&'static str`.
/// - If there are several labels, they must be specified as a tuple: `(&'static str, u16)`.
/// - The number of labels must match the number of label names and the constant param of `LabeledFamily`
///   (which is set to 1 by default). E.g., for two labels you should use `LabeledFamily<_, _, 2>`.
///
/// # Examples
///
/// ## Family with single label
///
/// ```
/// use vise::{Counter, LabeledFamily, Metrics};
/// # use vise::{Format, Registry};
///
/// #[derive(Debug, Metrics)]
/// struct TestMetrics {
///     #[metrics(labels = ["method"])]
///     counters: LabeledFamily<&'static str, Counter>,
/// }
///
/// // `counters` are keyed by a `&str`:
/// let metrics = TestMetrics::default();
/// metrics.counters[&"test"].inc();
/// metrics.counters[&"another_test"].inc_by(3);
/// // In the encoded metrics, these entries will be mentioned as follows:
/// let entries = [
///     r#"counters_total{method="test"} 1"#,
///     r#"counters_total{method="another_test"} 3"#,
/// ];
/// # let mut registry = Registry::empty();
/// # registry.register_metrics(&metrics);
/// # let mut buffer = String::new();
/// # registry.encode(&mut buffer, Format::OpenMetrics).unwrap();
/// # for entry in entries {
/// #     assert!(buffer.contains(&entry), "{buffer}");
/// # }
/// ```
///
/// ## Family with multiple labels
///
/// ```
/// # use vise::{Buckets, Format, Histogram, LabeledFamily, Metrics, Registry};
/// # use std::time::Duration;
/// const LABELS: [&str; 2] = ["method", "code"];
/// type Labels = (&'static str, u16);
///
/// #[derive(Debug, Metrics)]
/// struct TestMetrics {
///     #[metrics(labels = LABELS, buckets = Buckets::LATENCIES)]
///     latencies: LabeledFamily<Labels, Histogram<Duration>, 2>,
///     // ^ note that label names and type can be extracted elsewhere
/// }
///
/// let metrics = TestMetrics::default();
/// metrics.latencies[&("call", 200)].observe(Duration::from_millis(25));
/// metrics.latencies[&("send", 502)].observe(Duration::from_secs(1));
/// // In the encoded metrics, these entries will be mentioned as follows:
/// let entries = [
///     r#"latencies_sum{method="call",code="200"} 0.025"#,
///     r#"latencies_sum{method="send",code="502"} 1.0"#,
/// ];
/// # let mut registry = Registry::empty();
/// # registry.register_metrics(&metrics);
/// # let mut buffer = String::new();
/// # registry.encode(&mut buffer, Format::OpenMetrics).unwrap();
/// # for entry in entries {
/// #     assert!(buffer.contains(&entry), "{buffer}");
/// # }
/// ```
pub type LabeledFamily<S, M, const N: usize = 1> = Family<S, M, [&'static str; N]>;

impl<S, M, L> fmt::Debug for Family<S, M, L>
where
    S: fmt::Debug + Clone + Eq + Hash,
    M: BuildMetric + fmt::Debug,
    M::Builder: fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.inner, formatter)
    }
}

impl<S, M: BuildMetric, L: Clone> Clone for Family<S, M, L> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            labels: self.labels.clone(),
        }
    }
}

impl<S, M, L> Family<S, M, L>
where
    S: Clone + Eq + Hash,
    M: BuildMetric,
{
    pub(crate) fn new(builder: M::Builder, labels: L) -> Self {
        let inner = Arc::new(FamilyInner::new(builder));
        Self { inner, labels }
    }

    /// Checks whether this family contains a metric with the specified labels. This is mostly useful
    /// for testing.
    pub fn contains(&self, labels: &S) -> bool {
        self.inner.map.get(labels).is_some()
    }

    /// Gets a metric with the specified labels if it was reported previously. This is mostly useful
    /// for testing; use indexing for reporting.
    pub fn get(&self, labels: &S) -> Option<&M> {
        self.inner.map.get(labels)
    }

    /// Gets or creates a metric with the specified labels *lazily* (i.e., on first access). This is useful
    /// if the metric is updated conditionally and the condition is somewhat rare; in this case, indexing can
    /// unnecessarily blow up the number of metrics in the family.
    pub fn get_lazy(&self, labels: S) -> LazyItem<'_, S, M> {
        LazyItem::new(&self.inner, labels)
    }

    /// Returns all metrics currently present in this family together with the corresponding labels.
    /// This is inefficient and mostly useful for testing purposes.
    pub fn to_entries(&self) -> impl ExactSizeIterator<Item = (S, &M)> + '_ {
        self.inner.to_entries()
    }
}

/// Will create a new metric with the specified labels if it's missing in the family.
impl<S, M, L> ops::Index<&S> for Family<S, M, L>
where
    S: Clone + Eq + Hash,
    M: BuildMetric,
{
    type Output = M;

    fn index(&self, labels: &S) -> &Self::Output {
        self.inner.get_or_create(labels)
    }
}

/// Lazily accessed member of a [`Family`] or [`MetricsFamily`](crate::MetricsFamily). Returned
/// by `get_lazy()` methods.
pub struct LazyItem<'a, S, M: BuildMetric> {
    family: &'a FamilyInner<S, M>,
    labels: S,
}

impl<'a, S, M: BuildMetric> LazyItem<'a, S, M> {
    pub(crate) fn new(family: &'a FamilyInner<S, M>, labels: S) -> Self {
        Self { family, labels }
    }
}

impl<S, M> fmt::Debug for LazyItem<'_, S, M>
where
    S: fmt::Debug + Clone + Eq + Hash,
    M: BuildMetric + fmt::Debug,
    M::Builder: fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LazyItem")
            .field("family", self.family)
            .field("labels", &self.labels)
            .finish()
    }
}

impl<S: Clone, M: BuildMetric> Clone for LazyItem<'_, S, M> {
    fn clone(&self) -> Self {
        Self {
            family: self.family,
            labels: self.labels.clone(),
        }
    }
}

impl<S, M> ops::Deref for LazyItem<'_, S, M>
where
    S: Clone + Eq + Hash,
    M: BuildMetric,
{
    type Target = M;

    fn deref(&self) -> &Self::Target {
        self.family.get_or_create(&self.labels)
    }
}

impl<S, M, L> EncodeMetric for Family<S, M, L>
where
    M: BuildMetric + EncodeMetric + TypedMetric,
    S: Clone + Eq + Hash,
    L: MapLabels<S>,
{
    fn encode(&self, mut encoder: MetricEncoder<'_>) -> fmt::Result {
        for labels in &self.inner.map.keys_cloned() {
            let metric = self.inner.map.get(labels).unwrap();
            let mapped_labels = LabelSetWrapper(self.labels.map_labels(labels));
            let encoder = encoder.encode_family(&mapped_labels)?;
            metric.encode(encoder)?;
        }
        Ok(())
    }

    fn metric_type(&self) -> MetricType {
        <Self as TypedMetric>::TYPE
    }
}

impl<S, M: BuildMetric + TypedMetric, L> TypedMetric for Family<S, M, L> {
    const TYPE: MetricType = <M as TypedMetric>::TYPE;
}

impl<S, M, L> EncodeGroupedMetric for Family<S, M, L>
where
    M: BuildMetric + EncodeMetric + TypedMetric,
    S: Clone + Eq + Hash,
    L: MapLabels<S>,
{
    fn encode_grouped(
        &self,
        group_labels: &dyn EncodeLabelSet,
        encoder: &mut MetricEncoder<'_>,
    ) -> fmt::Result {
        for labels in &self.inner.map.keys_cloned() {
            let metric = self.inner.map.get(labels).unwrap();
            let mapped_labels = self.labels.map_labels(labels);
            let all_labels = FullLabelSet::new(group_labels, &mapped_labels);
            metric.encode(encoder.encode_family(&all_labels)?)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, thread};

    use prometheus_client::metrics::family::Family as StandardFamily;

    use super::*;
    use crate::MetricBuilder;

    type Label = (&'static str, &'static str);

    #[test]
    fn standard_family_is_easy_to_deadlock() {
        let (stop_sender, stop_receiver) = mpsc::channel();
        thread::spawn(move || {
            let family = StandardFamily::<Label, Gauge>::default();
            let first_metric = family.get_or_create(&("method", "test"));
            let second_metric = family.get_or_create(&("method", "other"));
            // ^ The second call will deadlock because of how `Family` is organized internally; its
            // `get_or_create()` provides a read guard for the internal map, and creating a new metric
            // requires a write lock on the same map.

            first_metric.set(10);
            second_metric.set(20);
            stop_sender.send(()).ok();
        });

        let err = stop_receiver
            .recv_timeout(Duration::from_millis(200))
            .unwrap_err();
        assert!(matches!(err, mpsc::RecvTimeoutError::Timeout));
    }

    #[test]
    fn family_accesses_are_not_deadlocked() {
        let family = Family::<Label, Gauge>::new(MetricBuilder::new(), ());
        let first_metric = &family[&("method", "test")];
        let second_metric = &family[&("method", "other")];
        first_metric.set(10);
        second_metric.set(20);

        // We circumvent deadlocking problems by using a *frozen map* (one that can be updated via a shared ref).
        // See its docs for more details. As an added bonus, we can use indexing notation instead of
        // clunky methods!
    }
}
