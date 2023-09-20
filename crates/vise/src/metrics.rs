//! Core `Metrics` trait defined by the crate.

use once_cell::sync::Lazy;

use std::ops;

use crate::{
    descriptors::MetricGroupDescriptor,
    registry::{CollectToRegistry, MetricsVisitor, Registry},
};

/// Collection of metrics for a library or application. Should be derived using the corresponding macro.
pub trait Metrics: 'static + Send + Sync {
    /// Metrics descriptor.
    const DESCRIPTOR: MetricGroupDescriptor;

    #[doc(hidden)] // implementation detail
    fn visit_metrics(&self, visitor: MetricsVisitor<'_>);
}

impl<M: Metrics> Metrics for &'static M {
    const DESCRIPTOR: MetricGroupDescriptor = M::DESCRIPTOR;

    fn visit_metrics(&self, visitor: MetricsVisitor<'_>) {
        (**self).visit_metrics(visitor);
    }
}

impl<M: Metrics> Metrics for Option<M> {
    const DESCRIPTOR: MetricGroupDescriptor = M::DESCRIPTOR;

    fn visit_metrics(&self, visitor: MetricsVisitor<'_>) {
        if let Some(metrics) = self {
            metrics.visit_metrics(visitor);
        }
    }
}

/// Global instance of [`Metrics`] allowing to access contained metrics from anywhere in code.
/// Should be used as a `static` item.
#[derive(Debug)]
pub struct Global<M: Metrics>(Lazy<M>);

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
    fn collect_to_registry(&'static self, registry: &mut Registry) {
        let metrics: &M = self;
        registry.register_metrics(metrics);
    }
}
