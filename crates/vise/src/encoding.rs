use prometheus_client::encoding::{EncodeMetric, LabelSetEncoder, MetricEncoder};
use std::fmt;

use crate::traits::EncodeLabelSet;

#[derive(Debug)]
pub(crate) struct LabelSetWrapper<S>(pub S);

impl<S: EncodeLabelSet> prometheus_client::encoding::EncodeLabelSet for LabelSetWrapper<S> {
    fn encode(&self, mut encoder: LabelSetEncoder<'_>) -> fmt::Result {
        self.0.encode(&mut encoder)
    }
}

#[derive(Default)]
pub(crate) struct LabelGroups(pub(crate) Vec<Box<dyn EncodeLabelSet>>);

impl fmt::Debug for LabelGroups {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LabelGroups")
            .field("len", &self.0.len())
            .finish()
    }
}

impl LabelGroups {
    const EMPTY: &'static Self = &Self(Vec::new());
}

#[derive(Debug)]
struct FullLabelSet<'a, S> {
    inner: &'a S,
    label_groups: &'a LabelGroups,
}

impl<S: EncodeLabelSet> prometheus_client::encoding::EncodeLabelSet for FullLabelSet<'_, S> {
    fn encode(&self, mut encoder: LabelSetEncoder<'_>) -> fmt::Result {
        for group in &self.label_groups.0 {
            group.encode(&mut encoder)?;
        }
        self.inner.encode(&mut encoder)
    }
}

#[derive(Debug)]
pub struct AdvancedMetricEncoder<'a> {
    inner: MetricEncoder<'a>,
    label_groups: &'a LabelGroups,
}

impl<'a> From<MetricEncoder<'a>> for AdvancedMetricEncoder<'a> {
    fn from(inner: MetricEncoder<'a>) -> Self {
        Self {
            inner,
            label_groups: LabelGroups::EMPTY,
        }
    }
}

impl<'a> AdvancedMetricEncoder<'a> {
    pub(crate) fn new(inner: MetricEncoder<'a>, label_groups: &'a LabelGroups) -> Self {
        Self {
            inner,
            label_groups,
        }
    }

    pub(crate) fn encode_family<'s>(
        &'s mut self,
        label_set: &'s impl EncodeLabelSet,
        action: impl FnOnce(MetricEncoder<'_>) -> fmt::Result,
    ) -> fmt::Result {
        let full_label_set = FullLabelSet {
            inner: label_set,
            label_groups: self.label_groups,
        };
        let encoder = self.inner.encode_family(&full_label_set)?;
        action(encoder)
    }
}

// TODO: docs, better name
pub trait AdvancedMetric: EncodeMetric {
    fn advanced_encode(&self, encoder: AdvancedMetricEncoder<'_>) -> fmt::Result {
        self.encode(encoder.inner)
    }
}
