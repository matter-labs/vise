//! Wrapper around metrics registry.

use linkme::distributed_slice;
use prometheus_client::{
    encoding::text,
    registry::{Descriptor, LocalMetric, Metric, Registry as RegistryInner, Unit},
};

use std::{collections::HashMap, fmt};

use crate::{
    collector::{Collector, LazyCollector},
    descriptors::{FullMetricDescriptor, MetricGroupDescriptor},
    format::{Format, PrometheusWrapper},
    Global, Metrics,
};

impl FullMetricDescriptor {
    fn format_for_panic(&self) -> String {
        format!(
            "{module}::{group_name}.{field_name} (line {line}) in crate {crate_name} {crate_version}",
            module = self.group.module_path,
            group_name = self.group.name,
            field_name = self.metric.field_name,
            line = self.group.line,
            crate_name = self.group.crate_name,
            crate_version = self.group.crate_version
        )
    }
}

/// Descriptors of all metrics in a registry.
#[derive(Debug, Default)]
pub struct RegisteredDescriptors {
    groups: Vec<&'static MetricGroupDescriptor>,
    metrics_by_name: HashMap<String, FullMetricDescriptor>,
}

impl RegisteredDescriptors {
    /// Iterates over descriptors for all registered groups.
    pub fn groups(&self) -> impl ExactSizeIterator<Item = &MetricGroupDescriptor> + '_ {
        self.groups.iter().copied()
    }

    /// Obtains a metric by its full name (i.e., the name reported to Prometheus).
    pub fn metric(&self, full_name: &str) -> Option<FullMetricDescriptor> {
        self.metrics_by_name.get(full_name).copied()
    }

    /// Returns the total number of registered metrics.
    pub fn metric_count(&self) -> usize {
        self.groups.iter().map(|group| group.metrics.len()).sum()
    }

    fn push(&mut self, group: &'static MetricGroupDescriptor) {
        for field in group.metrics {
            let descriptor = FullMetricDescriptor::new(group, field);
            let metric_name = field.full_name();
            if let Some(prev_descriptor) =
                self.metrics_by_name.insert(metric_name.clone(), descriptor)
            {
                panic!(
                    "Metric `{metric_name}` is redefined. New definition is at {descriptor}, \
                     previous definition was at {prev_descriptor}",
                    descriptor = descriptor.format_for_panic(),
                    prev_descriptor = prev_descriptor.format_for_panic()
                );
            }
        }
        self.groups.push(group);
    }
}

/// Configures collection of [`register`](crate::register)ed metrics.
#[derive(Debug)]
pub struct MetricsCollection<F = fn(&MetricGroupDescriptor) -> bool> {
    is_lazy: bool,
    filter_fn: F,
}

impl Default for MetricsCollection {
    fn default() -> Self {
        Self {
            is_lazy: false,
            filter_fn: |_| true,
        }
    }
}

impl MetricsCollection {
    /// Specifies that metrics should be lazily exported.
    ///
    /// FIXME: explain in more detail
    pub fn lazy() -> Self {
        Self {
            is_lazy: true,
            ..Self::default()
        }
    }

    /// Configures a filtering function for this collection.
    pub fn filter<F>(self, filter_fn: F) -> MetricsCollection<F>
    where
        F: FnMut(&MetricGroupDescriptor) -> bool,
    {
        MetricsCollection {
            is_lazy: self.is_lazy,
            filter_fn,
        }
    }
}

impl<F: FnMut(&MetricGroupDescriptor) -> bool> MetricsCollection<F> {
    /// Creates a registry with all [`register`](crate::register)ed [`Metrics`](crate::Metrics) implementations
    /// and [`Collector`]s.
    pub fn collect(mut self) -> Registry {
        let mut registry = Registry::empty();
        registry.is_lazy = self.is_lazy;
        for metric in METRICS_REGISTRATIONS {
            if (self.filter_fn)(metric.descriptor()) {
                metric.collect_to_registry(&mut registry);
            }
        }
        registry
    }
}

/// Metrics registry.
///
/// A registry collects [`Metrics`] and [`Collector`]s defined in an app and libs the app depends on.
/// Then, these metrics can be scraped by calling [`Self::encode()`] or [`Self::encode_to_text()`].
///
/// # Collecting metrics
///
/// You can include [`Metrics`] and [`Collector`]s to a registry manually using [`Self::register_metrics()`]
/// and [`Self::register_collector()`]. However, this can become untenable for large apps
/// with a complex dependency graph. As an alternative, you may use [`register`](crate::register) attributes
/// to mark [`Metrics`] and [`Collector`]s that should be present in the registry, and then initialize the registry
/// with [`Self::collect()`].
///
/// ```
/// use vise::{Buckets, Global, Histogram, Metrics, Registry, Unit};
/// # use assert_matches::assert_matches;
/// use std::time::Duration;
///
/// #[derive(Debug, Metrics)]
/// #[metrics(prefix = "my_app")]
/// pub(crate) struct AppMetrics {
///     /// Latency of requests served by the app.
///     #[metrics(buckets = Buckets::LATENCIES, unit = Unit::Seconds)]
///     pub request_latency: Histogram<Duration>,
/// }
///
/// #[vise::register]
/// // ^ Registers this instance for use with `Registry::collect()`
/// pub(crate) static APP_METRICS: Global<AppMetrics> = Global::new();
///
/// let registry = Registry::collect();
/// // Check that the registered metric is present
/// let descriptor = registry
///     .descriptors()
///     .metric("my_app_request_latency_seconds")
///     .unwrap();
/// assert_eq!(descriptor.metric.help, "Latency of requests served by the app");
/// assert_matches!(descriptor.metric.unit, Some(Unit::Seconds));
/// ```
///
/// `collect()` will panic if a metric is redefined:
///
/// ```should_panic
/// # use vise::{Collector, Global, Gauge, Metrics, Registry, Unit};
/// #[derive(Debug, Metrics)]
/// pub(crate) struct AppMetrics {
///     #[metrics(unit = Unit::Bytes)]
///     cache_memory_use: Gauge<u64>,
/// }
///
/// #[vise::register]
/// pub(crate) static APP_METRICS: Global<AppMetrics> = Global::new();
///
/// // Collector that provides the same (already registered!) metrics. This is
/// // logically incorrect; don't do this.
/// #[vise::register]
/// pub(crate) static APP_COLLECTOR: Collector<AppMetrics> = Collector::new();
///
/// let registry = Registry::collect(); // will panic
/// ```
#[derive(Debug)]
pub struct Registry {
    descriptors: RegisteredDescriptors,
    inner: RegistryInner,
    is_lazy: bool,
}

impl Registry {
    /// Creates an empty registry.
    pub fn empty() -> Self {
        Self {
            descriptors: RegisteredDescriptors::default(),
            inner: RegistryInner::default(),
            is_lazy: false,
        }
    }

    /// Returns descriptors for all registered metrics.
    pub fn descriptors(&self) -> &RegisteredDescriptors {
        &self.descriptors
    }

    /// Registers a group of metrics.
    pub fn register_metrics<M: Metrics>(&mut self, metrics: &M) {
        self.descriptors.push(&M::DESCRIPTOR);
        let visitor = MetricsVisitor(MetricsVisitorInner::Registry(self));
        metrics.visit_metrics(visitor);
    }

    pub(crate) fn register_global_metrics<M: Metrics>(&mut self, metrics: &'static Global<M>) {
        if self.is_lazy {
            self.descriptors.push(&M::DESCRIPTOR);
            let collector = LazyCollector::new(metrics);
            self.inner.register_collector(Box::new(collector));
        } else {
            self.register_metrics::<M>(metrics);
        }
    }

    /// Registers a [`Collector`].
    pub fn register_collector<M: Metrics>(&mut self, collector: &'static Collector<M>) {
        self.descriptors.push(&M::DESCRIPTOR);
        self.inner.register_collector(Box::new(collector));
    }

    /// Encodes all metrics in this registry to the specified text format.
    ///
    /// # Errors
    ///
    /// Proxies formatting errors of the provided `writer`.
    pub fn encode<W: fmt::Write>(&self, writer: &mut W, format: Format) -> fmt::Result {
        match format {
            Format::Prometheus | Format::OpenMetricsForPrometheus => {
                let remove_eof_terminator = matches!(format, Format::Prometheus);
                let mut wrapper = PrometheusWrapper::new(writer, remove_eof_terminator);
                text::encode(&mut wrapper, &self.inner)?;
                wrapper.flush()
            }
            Format::OpenMetrics => text::encode(writer, &self.inner),
        }
    }
}

#[derive(Debug)]
enum MetricsVisitorInner<'a> {
    Registry(&'a mut Registry),
    Collector(&'a mut Vec<(Descriptor, Box<dyn LocalMetric>)>),
}

/// Visitor for a group of metrics in a [`Registry`].
#[derive(Debug)]
pub struct MetricsVisitor<'a>(MetricsVisitorInner<'a>);

impl<'a> MetricsVisitor<'a> {
    pub(crate) fn for_collector(metrics: &'a mut Vec<(Descriptor, Box<dyn LocalMetric>)>) -> Self {
        Self(MetricsVisitorInner::Collector(metrics))
    }

    /// Registers a metric of family of metrics.
    pub fn push_metric(
        &mut self,
        name: &'static str,
        help: &'static str,
        unit: Option<Unit>,
        metric: impl Metric,
    ) {
        match &mut self.0 {
            MetricsVisitorInner::Registry(registry) => {
                if let Some(unit) = unit {
                    registry.inner.register_with_unit(name, help, unit, metric);
                } else {
                    registry.inner.register(name, help, metric);
                }
            }
            MetricsVisitorInner::Collector(collector) => {
                let descriptor = Descriptor::new(name, help, unit, None, vec![]);
                collector.push((descriptor, Box::new(metric)));
            }
        }
    }
}

/// Collects metrics from this type to registry. This is used by the [`register`](crate::register)
/// macro to handle registration of [`Global`] metrics and [`Collector`]s.
pub trait CollectToRegistry: 'static + Send + Sync {
    #[doc(hidden)] // implementation detail
    fn descriptor(&self) -> &'static MetricGroupDescriptor;
    #[doc(hidden)] // implementation detail
    fn collect_to_registry(&'static self, registry: &mut Registry);
}

#[doc(hidden)] // only used by the proc macros
#[distributed_slice]
pub static METRICS_REGISTRATIONS: [&'static dyn CollectToRegistry] = [..];
