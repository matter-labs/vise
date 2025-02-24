//! Core `Metrics` trait defined by the crate.

use once_cell::sync::Lazy;
use std::hash::Hash;

use std::ops;

use crate::{
    descriptors::MetricGroupDescriptor,
    registry::{CollectToRegistry, MetricsVisitor, Registry},
    traits::EncodeLabelSet,
    Family,
};

/// Collection of metrics for a library or application. Should be derived using the corresponding macro.
pub trait Metrics: 'static + Send + Sync {
    /// Metrics descriptor.
    const DESCRIPTOR: MetricGroupDescriptor;

    #[doc(hidden)] // implementation detail
    fn visit_metrics(&self, visitor: &mut MetricsVisitor<'_>);
}

impl<M: Metrics> Metrics for &'static M {
    const DESCRIPTOR: MetricGroupDescriptor = M::DESCRIPTOR;

    fn visit_metrics(&self, visitor: &mut MetricsVisitor<'_>) {
        (**self).visit_metrics(visitor);
    }
}

impl<M: Metrics> Metrics for Option<M> {
    const DESCRIPTOR: MetricGroupDescriptor = M::DESCRIPTOR;

    fn visit_metrics(&self, visitor: &mut MetricsVisitor<'_>) {
        if let Some(metrics) = self {
            metrics.visit_metrics(visitor);
        }
    }
}

impl<S, M> Metrics for Family<S, M>
where
    S: EncodeLabelSet + Clone + Eq + Hash + Send + Sync + 'static,
    M: Metrics + Default,
{
    const DESCRIPTOR: MetricGroupDescriptor = M::DESCRIPTOR;

    fn visit_metrics(&self, visitor: &mut MetricsVisitor<'_>) {
        for (labels, metrics) in self.to_entries() {
            visitor.push_group_labels(labels);
            metrics.visit_metrics(visitor);
            visitor.pop_group_labels();
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
        registry.register_global_metrics(self);
    }
}
