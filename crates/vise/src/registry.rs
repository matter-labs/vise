//! Wrapper around metrics registry.

use linkme::distributed_slice;
use prometheus_client::{
    encoding::text,
    registry::{Descriptor, LocalMetric, Metric, Registry as RegistryInner, Unit},
};

use std::{fmt, io};

use crate::{collector::Collector, Metrics};

#[doc(hidden)] // only used by the proc macros
#[distributed_slice]
pub static METRICS_REGISTRATIONS: [fn(&mut Registry)] = [..];

/// Metrics registry.
#[derive(Debug)]
pub struct Registry {
    inner: RegistryInner,
}

impl Registry {
    /// Creates an empty registry.
    pub fn empty() -> Self {
        Self {
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

    /// Registers a group of metrics.
    // TODO: collect metadata (defining crate, location etc.)?
    pub fn register_metrics<M: Metrics>(&mut self, metrics: &M) {
        let visitor = MetricsVisitor(MetricsVistorInner::Registry(self));
        metrics.visit_metrics(visitor);
    }

    /// Registers a [`Collector`].
    pub fn register_collector<M: Metrics>(&mut self, collector: &'static Collector<M>) {
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
    // TODO: check no redefinitions
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
