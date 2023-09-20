//! Metric descriptors.

use prometheus_client::{metrics::MetricType, registry::Unit};

/// Descriptor for a single metric.
#[derive(Debug)]
pub struct MetricDescriptor {
    /// Name of the metric **excluding** the unit suffix.
    pub name: &'static str,
    /// Name of the field in the metric group defining the metric.
    pub field_name: &'static str,
    /// Type of the metric.
    pub metric_type: MetricType,
    /// Measurement unit of the metric, if any.
    pub unit: Option<Unit>,
    /// Help for the metrics exported to Prometheus.
    pub help: &'static str,
    // TODO: labels?
}

impl MetricDescriptor {
    pub(crate) fn full_name(&self) -> String {
        if let Some(unit) = &self.unit {
            format!("{}_{}", self.name, unit.as_str())
        } else {
            self.name.to_owned()
        }
    }
}

/// Descriptor for a group of metrics (i.e., a struct implementing [`Metrics`](crate::Metrics)).
#[derive(Debug)]
pub struct MetricGroupDescriptor {
    /// Name of the crate in which the group is defined.
    pub crate_name: &'static str,
    /// Version of the crate (more precisely, the package that the crate is a part of).
    pub crate_version: &'static str,
    /// Path to the module in which the group is defined, e.g. `my_app::metrics`.
    pub module_path: &'static str,
    /// Name of the struct, e.g. `MyMetrics`.
    pub name: &'static str,
    /// Source code line on which the group is defined.
    pub line: u32,
    /// Descriptors of all metrics defined in the group.
    pub metrics: &'static [MetricDescriptor],
}

/// A metric descriptor together with a descriptor for a group in which the metric is defined.
#[derive(Debug, Clone, Copy)]
pub struct FullMetricDescriptor {
    /// Group descriptor.
    pub group: &'static MetricGroupDescriptor,
    /// Metric descriptor.
    pub metric: &'static MetricDescriptor,
}

impl FullMetricDescriptor {
    pub(crate) fn new(
        group: &'static MetricGroupDescriptor,
        metric: &'static MetricDescriptor,
    ) -> Self {
        Self { group, metric }
    }
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;

    use std::collections::HashMap;

    use super::*;
    use crate::{metrics::Metrics, tests::TestMetrics};

    #[test]
    fn describing_metrics() {
        let descriptor = TestMetrics::DESCRIPTOR;
        assert_eq!(descriptor.crate_name, "vise");
        assert_eq!(descriptor.module_path, "vise::tests");
        assert_eq!(descriptor.name, "TestMetrics");

        let metric_descriptors: HashMap<_, _> = descriptor
            .metrics
            .iter()
            .map(|descriptor| (descriptor.field_name, descriptor))
            .collect();

        let gauge_descriptor = metric_descriptors["gauge"];
        assert_matches!(gauge_descriptor.metric_type, MetricType::Gauge);
        assert_matches!(gauge_descriptor.unit, Some(Unit::Bytes));
        assert_eq!(gauge_descriptor.help, "");

        let histogram_descriptor = metric_descriptors["histogram"];
        assert_matches!(histogram_descriptor.metric_type, MetricType::Histogram);
        assert_matches!(histogram_descriptor.unit, None);
        assert_eq!(
            histogram_descriptor.help,
            "Histogram with inline bucket specification"
        );
    }
}
