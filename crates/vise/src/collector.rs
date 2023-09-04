use once_cell::sync::OnceCell;
use prometheus_client::{
    collector::Collector as CollectorTrait,
    registry::{Descriptor, LocalMetric},
    MaybeOwned,
};

use std::{borrow::Cow, error, fmt, iter};

use crate::{registry::MetricsVisitor, Metrics};

type CollectorFn<M> = Box<dyn Fn() -> M + Send + Sync>;
type CollectorItem<'a> = (Cow<'a, Descriptor>, MaybeOwned<'a, Box<dyn LocalMetric>>);

/// Error that can occur when calling [`Collector::before_scrape()`].
#[derive(Debug)]
pub struct BeforeScrapeError(());

impl fmt::Display for BeforeScrapeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Cannot set collector function: it is already set")
    }
}

impl error::Error for BeforeScrapeError {}

/// Collector allowing to define metrics dynamically.
///
/// In essence, a collector is a lazily initialized closure producing [`Metrics`] which is called
/// each time a [`Registry`](crate::Registry) it's registered with is being scraped.
///
/// ## Sharing state
///
/// Because of lazy initialization, the collector closure has access to the shared state with the rest
/// of the app. **Beware that `Collector`s live indefinitely.** To avoid resource leaks, use [`Weak`]
/// or similar types that do not lengthen the lifetime of the app state. Because `Metric` is
/// implemented for `Option`s, if the tracked state is dropped, you can simply return `None`
/// from the closure.
///
/// [`Weak`]: std::sync::Weak
pub struct Collector<M> {
    inner: OnceCell<CollectorFn<M>>,
}

impl<M> fmt::Debug for Collector<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Collector")
            .field("inner", &self.inner.get().map(|_| "_"))
            .finish()
    }
}

impl<M: Metrics> Collector<M> {
    /// Creates a new collector.
    pub const fn new() -> Self {
        Self {
            inner: OnceCell::new(),
        }
    }

    /// Initializes the producing function for this collector. The function will be called each time
    /// a [`Registry`] the collector is registered in is scraped.
    ///
    /// # Errors
    ///
    /// Returns an error if the producing function has been already set.
    pub fn before_scrape<F>(&'static self, hook: F) -> Result<(), BeforeScrapeError>
    where
        F: Fn() -> M + 'static + Send + Sync,
    {
        self.inner
            .set(Box::new(hook))
            .map_err(|_| BeforeScrapeError(()))
    }
}

impl<M: Metrics> CollectorTrait for &'static Collector<M> {
    fn collect<'a>(&'a self) -> Box<dyn Iterator<Item = CollectorItem<'a>> + 'a> {
        if let Some(hook) = self.inner.get() {
            let metrics = hook();
            let mut boxed_metrics = vec![];
            metrics.visit_metrics(MetricsVisitor::for_collector(&mut boxed_metrics));
            let it = boxed_metrics
                .into_iter()
                .map(|(descriptor, metric)| (Cow::Owned(descriptor), MaybeOwned::Owned(metric)));
            Box::new(it)
        } else {
            Box::new(iter::empty())
        }
    }
}

#[cfg(test)]
mod tests {
    use once_cell::sync::Lazy;

    use std::sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    };

    use super::*;
    use crate::{Gauge, Registry, Unit};

    #[derive(Debug, Metrics)]
    #[metrics(crate = crate, prefix = "dynamic")]
    struct TestMetrics {
        /// Test gauge.
        #[metrics(unit = Unit::Bytes)]
        gauge: Gauge,
    }

    /// Collector that produces owned metrics (useful if resource consumption is a concern).
    /// Metrics are also `Option`al to account for a potentially dropped data source.
    #[crate::register]
    #[metrics(crate = crate)]
    static OWNING_COLLECTOR: Collector<Option<TestMetrics>> = Collector::new();

    #[test]
    fn using_owning_collector() {
        let state = Arc::new(AtomicI64::new(0));
        let state_for_collector = Arc::downgrade(&state);

        OWNING_COLLECTOR
            .before_scrape(move || {
                let state = state_for_collector.upgrade()?;
                let metrics = TestMetrics::default();
                metrics.gauge.set(state.load(Ordering::Relaxed));
                Some(metrics)
            })
            .unwrap();

        let mut registry = Registry::empty();
        registry.register_collector(&OWNING_COLLECTOR);
        assert_collector_works(&registry, &state);

        drop(state);
        let mut buffer = String::new();
        registry.encode_to_text(&mut buffer).unwrap();

        assert_eq!(buffer, "# EOF\n");
    }

    fn assert_collector_works(registry: &Registry, state: &Arc<AtomicI64>) {
        state.store(123, Ordering::Release);
        let mut buffer = String::new();
        registry.encode_to_text(&mut buffer).unwrap();
        let lines: Vec<_> = buffer.lines().collect();

        let expected_lines = [
            "# HELP dynamic_gauge_bytes Test gauge.",
            "# TYPE dynamic_gauge_bytes gauge",
            "# UNIT dynamic_gauge_bytes bytes",
            "dynamic_gauge_bytes 123",
        ];
        for line in expected_lines {
            assert!(expected_lines.contains(&line), "{lines:#?}");
        }
    }

    /// Collector that produces borrowed metrics (useful if we want to update *some* of the metrics
    /// outside scraping / conditionally).
    static BORROWING_COLLECTOR: Collector<&'static TestMetrics> = Collector::new();

    /// Source of the collector.
    static METRICS_INSTANCE: Lazy<TestMetrics> = Lazy::new(TestMetrics::default);

    #[test]
    fn using_borrowing_collector() {
        let state = Arc::new(AtomicI64::new(0));
        let state_for_collector = Arc::downgrade(&state);

        BORROWING_COLLECTOR
            .before_scrape(move || {
                let metrics = &METRICS_INSTANCE;
                if let Some(state) = state_for_collector.upgrade() {
                    metrics.gauge.set(state.load(Ordering::Relaxed));
                }
                metrics
            })
            .unwrap();

        let mut registry = Registry::empty();
        registry.register_collector(&BORROWING_COLLECTOR);
        assert_collector_works(&registry, &state);

        METRICS_INSTANCE.gauge.set(42);
        drop(state);

        let mut buffer = String::new();
        registry.encode_to_text(&mut buffer).unwrap();
        let lines: Vec<_> = buffer.lines().collect();
        assert!(lines.contains(&"dynamic_gauge_bytes 42"), "{lines:#?}");
    }
}
