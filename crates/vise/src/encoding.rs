use std::{collections::HashMap, fmt, sync::Arc};

use prometheus_client::{
    encoding::{EncodeMetric, LabelSetEncoder, MetricEncoder},
    metrics::MetricType,
    registry::{Metric, Unit},
};

use crate::{traits::EncodeLabelSet, MetricsVisitor};

/// Wraps a label set so that it can be used in the `prometheus_client` library.
#[derive(Debug)]
pub(crate) struct LabelSetWrapper<S>(pub S);

impl<S: EncodeLabelSet> prometheus_client::encoding::EncodeLabelSet for LabelSetWrapper<S> {
    fn encode(&self, mut encoder: LabelSetEncoder<'_>) -> fmt::Result {
        self.0.encode(&mut encoder)
    }
}

#[derive(Default)]
struct LabeledMetric(Vec<(Arc<dyn EncodeLabelSet>, Box<dyn GroupedMetric>)>);

impl fmt::Debug for LabeledMetric {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LabeledMetric")
            .field("len", &self.0.len())
            .finish()
    }
}

impl EncodeMetric for LabeledMetric {
    fn encode(&self, mut encoder: MetricEncoder<'_>) -> fmt::Result {
        for (labels, metric) in &self.0 {
            metric
                .as_ref()
                .encode_grouped(labels.as_ref(), &mut encoder)?;
        }
        Ok(())
    }

    fn metric_type(&self) -> MetricType {
        self.0
            .first()
            .map_or(MetricType::Unknown, |(_, metric)| metric.metric_type())
    }
}

impl EncodeGroupedMetric for LabeledMetric {
    fn encode_grouped(
        &self,
        group_labels: &dyn EncodeLabelSet,
        encoder: &mut MetricEncoder<'_>,
    ) -> fmt::Result {
        for (labels, metric) in &self.0 {
            let all_labels = FullLabelSet::new(group_labels, labels.as_ref());
            metric.as_ref().encode_grouped(&all_labels, encoder)?;
        }
        Ok(())
    }
}

#[derive(Debug)]
struct MetricsGroup {
    help: &'static str,
    unit: Option<Unit>,
    instances: LabeledMetric,
}

/// Buffer for metrics in a `MetricsFamily`. Allows collecting metrics from all groups in the family,
/// so that they can be encoded grouped by metric (as opposed to by the group label set).
#[derive(Default)]
pub(crate) struct LabelGroups {
    labels: Option<Arc<dyn EncodeLabelSet>>,
    metrics_by_name: HashMap<&'static str, MetricsGroup>,
}

impl fmt::Debug for LabelGroups {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LabelGroups")
            .field("metrics_by_name", &self.metrics_by_name)
            .finish_non_exhaustive()
    }
}

impl MetricsVisitor for LabelGroups {
    fn visit_metric(
        &mut self,
        name: &'static str,
        help: &'static str,
        unit: Option<Unit>,
        metric: Box<dyn GroupedMetric>,
    ) {
        let metric_entry = self
            .metrics_by_name
            .entry(name)
            .or_insert_with(|| MetricsGroup {
                help,
                unit,
                instances: LabeledMetric::default(),
            });
        let current_labels = self
            .labels
            .clone()
            .expect("`LabelGroups` misused: group labels must be set before visiting metrics");
        metric_entry.instances.0.push((current_labels, metric));
    }
}

impl LabelGroups {
    pub(crate) fn set_labels(&mut self, labels: Arc<dyn EncodeLabelSet>) {
        self.labels = Some(labels);
    }

    pub(crate) fn visit_metrics(self, visitor: &mut dyn MetricsVisitor) {
        for (name, grouped_metric) in self.metrics_by_name {
            visitor.visit_metric(
                name,
                grouped_metric.help,
                grouped_metric.unit,
                Box::new(grouped_metric.instances),
            );
        }
    }
}

pub(crate) struct FullLabelSet<'a> {
    group_labels: &'a dyn EncodeLabelSet,
    inner: &'a dyn EncodeLabelSet,
}

impl fmt::Debug for FullLabelSet<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FullLabelSet")
            .finish_non_exhaustive()
    }
}

impl<'a> FullLabelSet<'a> {
    pub(crate) fn new(group_labels: &'a dyn EncodeLabelSet, inner: &'a dyn EncodeLabelSet) -> Self {
        Self {
            group_labels,
            inner,
        }
    }
}

impl EncodeLabelSet for FullLabelSet<'_> {
    fn encode(&self, encoder: &mut LabelSetEncoder<'_>) -> fmt::Result {
        self.group_labels.encode(encoder)?;
        self.inner.encode(encoder)
    }
}

impl prometheus_client::encoding::EncodeLabelSet for FullLabelSet<'_> {
    fn encode(&self, mut encoder: LabelSetEncoder<'_>) -> fmt::Result {
        EncodeLabelSet::encode(self, &mut encoder)
    }
}

/// Encodes a metric inside [a group](crate::MetricsFamily).
pub trait EncodeGroupedMetric: EncodeMetric {
    /// Performs encoding.
    fn encode_grouped(
        &self,
        labels: &dyn EncodeLabelSet,
        encoder: &mut MetricEncoder<'_>,
    ) -> fmt::Result {
        let labels = LabelSetWrapper(labels);
        self.encode(encoder.encode_family(&labels)?)
    }
}

/// [`EncodeGroupedMetric`] with additional constraints, such as `Send`, `Sync` and `'static` lifetime.
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
