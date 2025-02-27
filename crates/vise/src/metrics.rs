//! Core `Metrics` trait defined by the crate.

use std::{fmt, hash::Hash, ops, sync::Arc};

use once_cell::sync::Lazy;

use crate::{
    descriptors::MetricGroupDescriptor,
    encoding::LabelGroups,
    registry::{CollectToRegistry, MetricsVisitor, Registry},
    traits::EncodeLabelSet,
    wrappers::FamilyInner,
    LazyItem,
};

/// Collection of metrics for a library or application. Should be derived using the corresponding macro.
pub trait Metrics: 'static + Send + Sync {
    /// Metrics descriptor.
    const DESCRIPTOR: MetricGroupDescriptor;

    #[doc(hidden)] // implementation detail
    fn visit_metrics(&self, visitor: &mut dyn MetricsVisitor);
}

impl<M: Metrics> Metrics for &'static M {
    const DESCRIPTOR: MetricGroupDescriptor = M::DESCRIPTOR;

    fn visit_metrics(&self, visitor: &mut dyn MetricsVisitor) {
        (**self).visit_metrics(visitor);
    }
}

impl<M: Metrics> Metrics for Option<M> {
    const DESCRIPTOR: MetricGroupDescriptor = M::DESCRIPTOR;

    fn visit_metrics(&self, visitor: &mut dyn MetricsVisitor) {
        if let Some(metrics) = self {
            metrics.visit_metrics(visitor);
        }
    }
}

/// Global instance of [`Metrics`] allowing to access contained metrics from anywhere in code.
/// Should be used as a `static` item.
#[derive(Debug)]
pub struct Global<M: Metrics>(pub(crate) Lazy<M>);

impl<M: Metrics + Default> Default for Global<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: Metrics + Default> Global<M> {
    /// Creates a new metrics instance.
    pub const fn new() -> Self {
        Self(Lazy::new(M::default))
    }
}

impl<M: Metrics> ops::Deref for Global<M> {
    type Target = M;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<M: Metrics> CollectToRegistry for Global<M> {
    fn descriptor(&self) -> &'static MetricGroupDescriptor {
        &M::DESCRIPTOR
    }

    fn collect_to_registry(&'static self, registry: &mut Registry) {
        registry.register_global_metrics(&self.0, false);
    }
}

/// Family of [`Metrics`]. Allows applying one or more labels for all contained metrics, as if each of them was enclosed in a `Family`.
///
/// # Examples
///
/// ```
/// # use std::time::Duration;
/// # use derive_more::Display;
/// use vise::{
///     Buckets, Counter, Histogram, Metrics, MetricsFamily,
///     EncodeLabelValue, EncodeLabelSet,
/// };
///
/// #[derive(Debug, Metrics)]
/// #[metrics(prefix = "rpc_method")]
/// struct MethodMetrics {
///     errors: Counter,
///     #[metrics(buckets = Buckets::LATENCIES)]
///     latency: Histogram<Duration>,
/// }
///
/// #[derive(Debug, Display, Clone, Copy, PartialEq, Eq, Hash)]
/// #[derive(EncodeLabelValue, EncodeLabelSet)]
/// #[metrics(label = "method")]
/// struct Method(&'static str);
///
/// #[vise::register]
/// static METRICS: MetricsFamily<Method, MethodMetrics> = MetricsFamily::new();
///
/// // Metrics can be accessed via indexing:
/// METRICS[&Method("eth_call")].errors.inc();
/// METRICS[&Method("eth_blockNumber")].latency.observe(Duration::from_millis(100));
///
/// // Metric instances can be stashed in a struct if needed; they have static lifetime.
/// let call_metrics: &'static MethodMetrics = &METRICS[&Method("eth_call")];
/// call_metrics.errors.inc_by(2);
///
/// let registry = vise::MetricsCollection::default().collect();
/// let mut buffer = String::new();
/// registry.encode(&mut buffer, vise::Format::OpenMetrics)?;
/// assert!(buffer
///     .lines()
///     .any(|line| line == r#"rpc_method_errors_total{method="eth_call"} 3"#));
/// # Ok::<_, std::fmt::Error>(())
/// ```
pub struct MetricsFamily<S, M: Metrics + Default>(Lazy<FamilyInner<S, M>>);

impl<S, M> fmt::Debug for MetricsFamily<S, M>
where
    S: Clone + Eq + Hash + fmt::Debug,
    M: Metrics + Default + fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, formatter)
    }
}

impl<S, M> Default for MetricsFamily<S, M>
where
    S: Clone + Eq + Hash,
    M: Metrics + Default,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<S, M> MetricsFamily<S, M>
where
    S: Clone + Eq + Hash,
    M: Metrics + Default,
{
    /// Creates a new metrics family.
    pub const fn new() -> Self {
        Self(Lazy::new(|| FamilyInner::new(())))
    }

    /// Gets or creates metrics with the specified labels *lazily* (i.e., on first access). This is useful
    /// if the metrics are updated conditionally and the condition is somewhat rare; in this case, indexing can
    /// unnecessarily blow up the number of metrics in the family.
    pub fn get_lazy(&self, labels: S) -> LazyItem<'_, S, M> {
        LazyItem::new(&self.0, labels)
    }

    /// Returns all metrics currently present in this family together with the corresponding labels.
    /// This is inefficient and mostly useful for testing purposes.
    pub fn to_entries(&self) -> impl ExactSizeIterator<Item = (S, &M)> + '_ {
        self.0.to_entries()
    }
}

impl<S, M> ops::Index<&S> for MetricsFamily<S, M>
where
    S: Clone + Eq + Hash,
    M: Metrics + Default,
{
    type Output = M;

    fn index(&self, labels: &S) -> &Self::Output {
        self.0.get_or_create(labels)
    }
}

impl<S, M> Metrics for FamilyInner<S, M>
where
    S: EncodeLabelSet + Clone + Eq + Hash + Send + Sync + 'static,
    M: Metrics + Default,
{
    const DESCRIPTOR: MetricGroupDescriptor = M::DESCRIPTOR;

    fn visit_metrics(&self, visitor: &mut dyn MetricsVisitor) {
        let mut grouped = LabelGroups::default();
        for (labels, metrics) in self.to_entries() {
            grouped.set_labels(Arc::new(labels));
            metrics.visit_metrics(&mut grouped);
        }
        grouped.visit_metrics(visitor);
    }
}

impl<S, M> Metrics for MetricsFamily<S, M>
where
    S: EncodeLabelSet + Clone + Eq + Hash + Send + Sync + 'static,
    M: Metrics + Default,
{
    const DESCRIPTOR: MetricGroupDescriptor = M::DESCRIPTOR;

    fn visit_metrics(&self, visitor: &mut dyn MetricsVisitor) {
        if let Some(inner) = Lazy::get(&self.0) {
            inner.visit_metrics(visitor);
        }
    }
}

impl<S, M> CollectToRegistry for MetricsFamily<S, M>
where
    S: EncodeLabelSet + Clone + Eq + Hash + Send + Sync + 'static,
    M: Metrics + Default,
{
    fn descriptor(&self) -> &'static MetricGroupDescriptor {
        &M::DESCRIPTOR
    }

    fn collect_to_registry(&'static self, registry: &mut Registry) {
        registry.register_global_metrics(&self.0, true);
    }
}
