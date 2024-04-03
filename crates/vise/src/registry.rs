//! Wrapper around metrics registry.

use linkme::distributed_slice;
use prometheus_client::{
    encoding::{text, DescriptorEncoder},
    registry::{Metric, Registry as RegistryInner, Unit},
};

use std::{collections::HashMap, fmt};

use crate::{
    collector::{Collector, LazyGlobalCollector},
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
///
/// # Examples
///
/// See [`Registry`] docs for examples of usage.
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
    /// By default, [`Global`] metrics are eagerly collected into a [`Registry`]; i.e., metrics will get exported
    /// even if they were never modified by the app / library logic. This is *usually* fine (e.g.,
    /// this allows getting all metrics metadata on the first scrape), but sometimes you may want to
    /// export only metrics touched by the app / library logic. E.g., you have a single app binary
    /// that exposes different sets of metrics depending on configuration, and exporting all metrics
    /// is confusing and/or unacceptably bloats exported data size.
    ///
    /// `lazy()` solves this issue. It will configure the created `Registry` so that `Global` metrics
    /// are only exported after they are touched by the app / library logic. Beware that this includes
    /// being touched by an eager `MetricsCollection` (only metrics actually included into the collection
    /// are touched).
    pub fn lazy() -> Self {
        Self {
            is_lazy: true,
            ..Self::default()
        }
    }

    /// Configures a filtering predicate for this collection. Only [`Metrics`] with a descriptor
    /// satisfying this will be [collected](MetricsCollection::collect()).
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
    /// Creates a registry with all [`register`](crate::register)ed [`Global`] metrics
    /// and [`Collector`]s. If a filtering predicate [was provided](MetricsCollection::filter()),
    /// only metrics satisfying this function will be collected.
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
/// Then, these metrics can be scraped by calling [`Self::encode()`].
///
/// # Collecting metrics
///
/// You can include [`Metrics`] and [`Collector`]s to a registry manually using [`Self::register_metrics()`]
/// and [`Self::register_collector()`]. However, this can become untenable for large apps
/// with a complex dependency graph. As an alternative, you may use [`register`](crate::register) attributes
/// to mark [`Metrics`] and [`Collector`]s that should be present in the registry, and then initialize the registry
/// with [`MetricsCollection`] methods.
///
/// ```
/// use vise::{Buckets, Global, Histogram, Metrics, MetricsCollection, Registry, Unit};
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
/// // ^ Registers this instance for use with `MetricsCollection::collect()`
/// pub(crate) static APP_METRICS: Global<AppMetrics> = Global::new();
///
/// let registry: Registry = MetricsCollection::default().collect();
/// // Check that the registered metric is present
/// let descriptor = registry
///     .descriptors()
///     .metric("my_app_request_latency_seconds")
///     .unwrap();
/// assert_eq!(descriptor.metric.help, "Latency of requests served by the app");
/// assert_matches!(descriptor.metric.unit, Some(Unit::Seconds));
/// ```
///
/// Registered metrics can be filtered. This is useful if you want to avoid exporting certain metrics
/// under certain conditions.
///
/// ```
/// # use vise::{MetricsCollection, Registry};
/// let filtered_registry: Registry = MetricsCollection::default()
///     .filter(|group| group.name == "AppMetrics")
///     .collect();
/// // Do something with `filtered_registry`...
/// ```
///
/// `collect()` will panic if a metric is redefined:
///
/// ```should_panic
/// # use vise::{Collector, Global, Gauge, Metrics, MetricsCollection, Unit};
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
/// let registry = MetricsCollection::default().collect(); // will panic
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
        let mut visitor = MetricsVisitor(MetricsVisitorInner::Registry(self));
        metrics.visit_metrics(&mut visitor);
    }

    pub(crate) fn register_global_metrics<M: Metrics>(&mut self, metrics: &'static Global<M>) {
        if self.is_lazy {
            self.descriptors.push(&M::DESCRIPTOR);
            let collector = LazyGlobalCollector::new(metrics);
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
                let mut wrapper = PrometheusWrapper::new(writer);
                if matches!(format, Format::Prometheus) {
                    wrapper.remove_eof_terminator();
                    wrapper.translate_info_metrics_type();
                }
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
    Collector(Result<DescriptorEncoder<'a>, fmt::Error>),
}

/// Visitor for a group of metrics in a [`Registry`].
#[derive(Debug)]
pub struct MetricsVisitor<'a>(MetricsVisitorInner<'a>);

impl<'a> MetricsVisitor<'a> {
    pub(crate) fn for_collector(encoder: DescriptorEncoder<'a>) -> Self {
        Self(MetricsVisitorInner::Collector(Ok(encoder)))
    }

    pub(crate) fn check(self) -> fmt::Result {
        match self.0 {
            MetricsVisitorInner::Registry(_) => Ok(()),
            MetricsVisitorInner::Collector(res) => res.map(drop),
        }
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
            MetricsVisitorInner::Collector(encode_result) => {
                if let Ok(encoder) = encode_result {
                    let new_result =
                        Self::encode_metric(encoder, name, help, unit.as_ref(), &metric);
                    if let Err(err) = new_result {
                        *encode_result = Err(err);
                    }
                }
            }
        }
    }

    fn encode_metric(
        encoder: &mut DescriptorEncoder<'_>,
        name: &'static str,
        help: &'static str,
        unit: Option<&Unit>,
        metric: &impl Metric,
    ) -> fmt::Result {
        // Append a full stop to `help` to be consistent with registered metrics.
        let mut help = String::from(help);
        help.push('.');

        let metric_encoder = encoder.encode_descriptor(name, &help, unit, metric.metric_type())?;
        metric.encode(metric_encoder)
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
