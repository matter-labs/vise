//! Wrapper around metrics registry.

use prometheus_client::{
    encoding::{text, DescriptorEncoder},
    registry::{Registry as RegistryInner, Unit},
};

use once_cell::sync::Lazy;
use std::sync::Mutex;
use std::{collections::HashMap, fmt};

use crate::encoding::GroupedMetric;
use crate::{
    collector::{Collector, LazyGlobalCollector},
    descriptors::{FullMetricDescriptor, MetricGroupDescriptor},
    format::{Format, PrometheusWrapper},
    Metrics,
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
    #[allow(clippy::missing_panics_doc)]
    pub fn collect(mut self) -> Registry {
        let mut registry = Registry::empty();
        registry.is_lazy = self.is_lazy;
        for metric in METRICS_REGISTRATIONS.get() {
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
        metrics.visit_metrics(self);
    }

    pub(crate) fn register_global_metrics<M: Metrics>(
        &mut self,
        metrics: &'static Lazy<M>,
        force_lazy: bool,
    ) {
        if force_lazy || self.is_lazy {
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

/// FIXME
pub trait MetricsVisitor {
    /// FIXME
    fn visit_metric(
        &mut self,
        name: &'static str,
        help: &'static str,
        unit: Option<Unit>,
        metric: Box<dyn GroupedMetric>,
    );
}

impl MetricsVisitor for Registry {
    fn visit_metric(
        &mut self,
        name: &'static str,
        help: &'static str,
        unit: Option<Unit>,
        metric: Box<dyn GroupedMetric>,
    ) {
        if let Some(unit) = unit {
            self.inner.register_with_unit(name, help, unit, metric);
        } else {
            self.inner.register(name, help, metric);
        }
    }
}

#[derive(Debug)]
pub(crate) struct MetricsEncoder<'a> {
    inner: Result<DescriptorEncoder<'a>, fmt::Error>,
}

impl MetricsEncoder<'_> {
    pub(crate) fn check(self) -> fmt::Result {
        self.inner.map(drop)
    }
}

impl<'a> From<DescriptorEncoder<'a>> for MetricsEncoder<'a> {
    fn from(inner: DescriptorEncoder<'a>) -> Self {
        Self { inner: Ok(inner) }
    }
}

impl MetricsVisitor for MetricsEncoder<'_> {
    fn visit_metric(
        &mut self,
        name: &'static str,
        help: &'static str,
        unit: Option<Unit>,
        metric: Box<dyn GroupedMetric>,
    ) {
        if let Ok(encoder) = &mut self.inner {
            // Append a full stop to `help` to be consistent with registered metrics.
            let mut help = String::from(help);
            help.push('.');

            let new_result = encoder
                .encode_descriptor(name, &help, unit.as_ref(), metric.metric_type())
                .and_then(|encoder| metric.encode(encoder));
            if let Err(err) = new_result {
                self.inner = Err(err);
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

// Intentionally not re-exported; used by the proc macros
pub struct MetricsRegistrations {
    inner: Mutex<Vec<&'static dyn CollectToRegistry>>,
}

impl fmt::Debug for MetricsRegistrations {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Ok(metrics) = self.inner.lock() {
            let descriptors = metrics.iter().map(|metrics| metrics.descriptor());
            formatter.debug_list().entries(descriptors).finish()
        } else {
            formatter
                .debug_tuple("MetricsRegistrations")
                .field(&"poisoned")
                .finish()
        }
    }
}

impl MetricsRegistrations {
    const fn new() -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
        }
    }

    // Only called by the `register` proc macro before main. `unwrap()` isn't expected to panic (panicking before main could lead to UB)
    // since it's just pushing a value into a `Vec`. If this becomes a concern, we could rework `MetricsRegistrations`
    // to use a lock-free linked list as in `inventory`: https://github.com/dtolnay/inventory/blob/f15e000224ca5d873097d406287bf79905f12c35/src/lib.rs#L190
    pub fn push(&self, metrics: &'static dyn CollectToRegistry) {
        self.inner.lock().unwrap().push(metrics);
    }

    fn get(&self) -> Vec<&'static dyn CollectToRegistry> {
        self.inner.lock().unwrap().clone()
    }
}

#[doc(hidden)] // only used by the proc macros
pub static METRICS_REGISTRATIONS: MetricsRegistrations = MetricsRegistrations::new();
