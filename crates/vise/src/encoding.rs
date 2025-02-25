use prometheus_client::encoding::{
    DescriptorEncoder, EncodeMetric, LabelSetEncoder, MetricEncoder,
};
use prometheus_client::metrics::MetricType;
use prometheus_client::registry::{Metric, Unit};
use std::collections::HashMap;
use std::fmt;

use crate::traits::EncodeLabelSet;

#[derive(Debug)]
pub(crate) struct LabelSetWrapper<S>(pub S);

impl<S: EncodeLabelSet> prometheus_client::encoding::EncodeLabelSet for LabelSetWrapper<S> {
    fn encode(&self, mut encoder: LabelSetEncoder<'_>) -> fmt::Result {
        self.0.encode(&mut encoder)
    }
}

struct GroupedMetricInstances {
    help: &'static str,
    unit: Option<Unit>,
    instances: Vec<(usize, Box<dyn EncodeGroupedMetric>)>,
}

impl fmt::Debug for GroupedMetricInstances {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GroupedMetric")
            .field("help", &self.help)
            .field("unit", &self.unit)
            .finish_non_exhaustive()
    }
}

/// Buffer for metrics in a `MetricsFamily`. Allows collecting metrics from all groups in the family,
/// so that they can be encoded grouped by metric (as opposed to by the group label set).
#[derive(Default)]
pub(crate) struct LabelGroups {
    labels: Vec<Box<dyn EncodeLabelSet>>,
    metrics_by_name: HashMap<&'static str, GroupedMetricInstances>,
}

impl fmt::Debug for LabelGroups {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LabelGroups")
            .field("labels", &self.labels.len())
            .field("metrics_by_name", &self.metrics_by_name)
            .finish()
    }
}

impl LabelGroups {
    pub(crate) fn push_labels(&mut self, labels: Box<dyn EncodeLabelSet>) {
        self.labels.push(labels);
    }

    pub(crate) fn push_metric(
        &mut self,
        name: &'static str,
        help: &'static str,
        unit: Option<&Unit>,
        metric: Box<dyn EncodeGroupedMetric>,
    ) {
        let metric_entry =
            self.metrics_by_name
                .entry(name)
                .or_insert_with(|| GroupedMetricInstances {
                    help,
                    unit: unit.cloned(),
                    instances: vec![],
                });
        let current_group_idx = self.labels.len() - 1;
        metric_entry.instances.push((current_group_idx, metric));
    }

    pub(crate) fn encode(self, encoder: &mut DescriptorEncoder<'_>) -> fmt::Result {
        for (name, grouped_metric) in self.metrics_by_name {
            let Some((_, metric)) = grouped_metric.instances.first() else {
                continue;
            };
            let instances = grouped_metric
                .instances
                .iter()
                .map(|(idx, metric)| (self.labels[*idx].as_ref(), metric.as_ref()));

            let metric_encoder = encoder.encode_descriptor(
                name,
                grouped_metric.help,
                grouped_metric.unit.as_ref(),
                metric.metric_type(),
            )?;
            let mut metric_encoder = GroupedMetricEncoder {
                inner: metric_encoder,
                group_labels: &(),
            };
            for (group_labels, instance) in instances {
                metric_encoder.group_labels = group_labels;
                instance.encode_grouped(&mut metric_encoder)?;
            }
        }
        Ok(())
    }
}

struct FullLabelSet<'a, S> {
    inner: &'a S,
    group_labels: &'a dyn EncodeLabelSet,
}

impl<S: fmt::Debug> fmt::Debug for FullLabelSet<'_, S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FullLabelSet")
            .field("inner", self.inner)
            .finish_non_exhaustive()
    }
}

impl<S: EncodeLabelSet> prometheus_client::encoding::EncodeLabelSet for FullLabelSet<'_, S> {
    fn encode(&self, mut encoder: LabelSetEncoder<'_>) -> fmt::Result {
        self.group_labels.encode(&mut encoder)?;
        self.inner.encode(&mut encoder)
    }
}

pub struct GroupedMetricEncoder<'a> {
    inner: MetricEncoder<'a>,
    group_labels: &'a dyn EncodeLabelSet,
}

impl fmt::Debug for GroupedMetricEncoder<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GroupedMetricEncoder")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

impl<'a> GroupedMetricEncoder<'a> {
    pub(crate) fn encode_family<'s>(
        &'s mut self,
        label_set: &'s impl EncodeLabelSet,
        action: impl FnOnce(MetricEncoder<'_>) -> fmt::Result,
    ) -> fmt::Result {
        let full_label_set = FullLabelSet {
            inner: label_set,
            group_labels: self.group_labels,
        };
        let encoder = self.inner.encode_family(&full_label_set)?;
        action(encoder)
    }
}

// FIXME: rework
/// Encodes a metric inside [a group](crate::MetricsFamily).
pub trait EncodeGroupedMetric: EncodeMetric {
    fn encode_grouped(&self, encoder: &mut GroupedMetricEncoder<'_>) -> fmt::Result {
        let labels = LabelSetWrapper(encoder.group_labels);
        let encoder = encoder.inner.encode_family(&labels)?;
        self.encode(encoder)
    }
}

/// FIXME
pub trait GroupedMetric: EncodeGroupedMetric + Metric {}

impl<T> GroupedMetric for T where T: EncodeGroupedMetric + Metric {}

impl EncodeMetric for Box<dyn GroupedMetric> {
    fn encode(&self, encoder: MetricEncoder<'_>) -> fmt::Result {
        (**self).encode(encoder)
    }

    fn metric_type(&self) -> MetricType {
        (**self).metric_type()
    }
}
