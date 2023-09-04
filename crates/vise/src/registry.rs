//! Wrapper around metrics registry.

use linkme::distributed_slice;
use prometheus_client::{
    encoding::text,
    registry::{Descriptor, LocalMetric, Metric, Registry as RegistryInner, Unit},
};

use std::{collections::HashMap, fmt, io};

use crate::{
    collector::Collector,
    descriptors::{FullMetricDescriptor, MetricGroupDescriptor},
    Metrics,
};

#[doc(hidden)] // only used by the proc macros
#[distributed_slice]
pub static METRICS_REGISTRATIONS: [fn(&mut Registry)] = [..];

impl FullMetricDescriptor {
    fn format_for_panic(&self) -> String {
        format!(
            "{module}::{group_name}.{field_name} (line {line})",
            module = self.group.module_path,
            group_name = self.group.name,
            field_name = self.metric.field_name,
            line = self.group.line
        )
    }
}

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

/// Metrics registry.
#[derive(Debug)]
pub struct Registry {
    descriptors: RegisteredDescriptors,
    inner: RegistryInner,
}

impl Registry {
    /// Creates an empty registry.
    pub fn empty() -> Self {
        Self {
            descriptors: RegisteredDescriptors::default(),
            inner: RegistryInner::default(),
        }
    }

    /// Creates a registry with all [`Metrics`](crate::Metrics) implementations automatically injected.
    // TODO: allow filtering metrics (by a descriptor predicate?)
    pub fn collect() -> Self {
        let mut this = Self::empty();
        for metric_fn in METRICS_REGISTRATIONS {
            (metric_fn)(&mut this);
        }
        this
    }

    /// Returns descriptors for all registered metrics.
    pub fn descriptors(&self) -> &RegisteredDescriptors {
        &self.descriptors
    }

    /// Registers a group of metrics.
    pub fn register_metrics<M: Metrics>(&mut self, metrics: &M) {
        self.descriptors.push(&M::DESCRIPTOR);
        let visitor = MetricsVisitor(MetricsVistorInner::Registry(self));
        metrics.visit_metrics(visitor);
    }

    /// Registers a [`Collector`].
    pub fn register_collector<M: Metrics>(&mut self, collector: &'static Collector<M>) {
        self.descriptors.push(&M::DESCRIPTOR);
        self.inner.register_collector(Box::new(collector));
    }

    /// Encodes all metrics in this registry using the Open Metrics text format.
    ///
    /// # Errors
    ///
    /// Proxies I/O errors of the provided `writer`.
    #[allow(clippy::missing_panics_doc)] // false positive
    pub fn encode<W: io::Write>(&self, writer: W) -> io::Result<()> {
        let mut writer = WriterWrapper::new(writer);
        text::encode(&mut writer, &self.inner).map_err(|_| writer.error.unwrap())
    }

    /// Encodes all metrics in this registry to the Open Metrics text format. Unlike [`Self::encode()`],
    /// this method accepts a string writer, not a byte one.
    ///
    /// # Errors
    ///
    /// Proxies formatting errors of the provided `writer`.
    pub fn encode_to_text<W: fmt::Write>(&self, writer: &mut W) -> fmt::Result {
        text::encode(writer, &self.inner)
    }
}

#[derive(Debug)]
struct WriterWrapper<W> {
    writer: W,
    error: Option<io::Error>,
}

impl<W: io::Write> WriterWrapper<W> {
    fn new(writer: W) -> Self {
        Self {
            writer,
            error: None,
        }
    }
}

impl<W: io::Write> fmt::Write for WriterWrapper<W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.writer.write_all(s.as_bytes()).map_err(|err| {
            self.error = Some(err);
            fmt::Error
        })
    }
}

#[derive(Debug)]
enum MetricsVistorInner<'a> {
    Registry(&'a mut Registry),
    Collector(&'a mut Vec<(Descriptor, Box<dyn LocalMetric>)>),
}

/// Registration for a group of metrics in a [`Registry`].
#[derive(Debug)]
pub struct MetricsVisitor<'a>(MetricsVistorInner<'a>);

impl<'a> MetricsVisitor<'a> {
    pub(crate) fn for_collector(metrics: &'a mut Vec<(Descriptor, Box<dyn LocalMetric>)>) -> Self {
        Self(MetricsVistorInner::Collector(metrics))
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
            MetricsVistorInner::Registry(registry) => {
                if let Some(unit) = unit {
                    registry.inner.register_with_unit(name, help, unit, metric);
                } else {
                    registry.inner.register(name, help, metric);
                }
            }
            MetricsVistorInner::Collector(collector) => {
                let descriptor = Descriptor::new(name, help, unit, None, vec![]);
                collector.push((descriptor, Box::new(metric)));
            }
        }
    }
}
