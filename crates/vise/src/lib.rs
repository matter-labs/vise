//! Metrics handling library based on the `prometheus-client` crate.
//!
//! # Overview
//!
//! - The crate supports defining common metric types ([`Counter`]s, [`Gauge`]s and [`Histogram`]s).
//!   A single metric is represented by an instance of these types; it can be reported using methods
//!   like [`Counter::inc()`], [`Gauge::set()`] or [`Histogram::observe()`].
//! - Metrics can be grouped into a [`Family`]. Essentially, a `Family` is a map in which metrics
//!   are values keyed by a set of labels. See [`EncodeLabelValue`] and [`EncodeLabelSet`] derive macros
//!   for more info on labels.
//! - To define metrics, a group of logically related metrics is grouped into a struct
//!   and the [`Metrics`](trait@Metrics) trait is [derived](macro@Metrics) for it. This resolves
//!   full metric names and records additional metadata, such as help (from doc comments), unit of measurement
//!   and [`Buckets`] for histograms.
//! - Metric groups are registered in a [`Registry`], which then allows to [encode](Registry::encode())
//!   metric data in the Open Metrics text format. Registration can be automated using the [`register`]
//!   attribute, but it can be manual as well.
//! - In order to allow for metrics computed during scraping, you can use [`Collector`].
//!
//! # Examples
//!
//! ## Defining metrics
//!
//! ```
//! use vise::*;
//! use std::{fmt, time::Duration};
//!
//! /// Metrics defined by the library or application. A single app / lib can define
//! /// multiple metric structs.
//! #[derive(Debug, Clone, Metrics)]
//! #[metrics(prefix = "my_app")]
//! // ^ Prefix added to all field names to get the final metric name (e.g., `my_app_latencies`).
//! pub(crate) struct MyMetrics {
//!     /// Simple counter. Doc comments for the fields will be reported
//!     /// as Prometheus metric descriptions.
//!     pub counter: Counter,
//!     /// Integer-valued gauge. Unit will be reported to Prometheus and will influence metric name
//!     /// by adding the corresponding suffix to it (in this case, `_bytes`).
//!     #[metrics(unit = Unit::Bytes)]
//!     pub gauge: Gauge<u64>,
//!     /// Group of histograms with the "method" label (see the definition below).
//!     /// Each `Histogram` or `Family` of `Histogram`s must define buckets; in this case,
//!     /// we use default buckets for latencies.
//!     #[metrics(buckets = Buckets::LATENCIES)]
//!     pub latencies: Family<Method, Histogram<Duration>>,
//! }
//!
//! /// Isolated metric label. Note the `label` name specification below.
//! #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelSet, EncodeLabelValue)]
//! #[metrics(label = "method")]
//! pub(crate) struct Method(pub &'static str);
//!
//! // For the isolated metric label to work, you should implement `Display` for it:
//! impl fmt::Display for Method {
//!     fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
//!         write!(formatter, "{}", self.0)
//!     }
//! }
//! ```
//!
//! ## Registering metrics automatically
//!
//! Commonly, metrics can be registered by defining a `static`:
//!
//! ```
//! # use vise::{Gauge, Global, Metrics, Registry};
//! #[derive(Debug, Clone, Metrics)]
//! pub(crate) struct MyMetrics {
//! #   pub gauge: Gauge<u64>,
//!     // defined in a previous code sample
//! }
//!
//! #[vise::register]
//! pub(crate) static MY_METRICS: Global<MyMetrics> = Global::new();
//!
//! // All registered metrics can be collected in a `Registry`:
//! let registry = Registry::collect();
//! // Do something with the `registry`, e.g. create an exporter.
//!
//! fn metered_logic() {
//!     // method logic...
//!     MY_METRICS.gauge.set(42);
//! }
//! ```
//!
//! ## Registering metrics manually
//!
//! It is possible to add metrics manually to a [`Registry`]. As a downside, this approach requires
//! boilerplate to register all necessary metrics in an app and potentially libraries
//! that it depends on.
//!
//! ```
//! # use vise::{Gauge, Metrics, Registry};
//! #[derive(Debug, Clone, Metrics)]
//! pub(crate) struct MyMetrics {
//! #   pub gauge: Gauge<u64>,
//!     // defined in a previous code sample
//! }
//!
//! let mut registry = Registry::empty();
//! let my_metrics = MyMetrics::default();
//! registry.register_metrics(&my_metrics);
//! // Do something with the `registry`, e.g. create an exporter.
//!
//! // After registration, metrics can be moved to logic that reports the metrics.
//! // Note that metric types trivially implement `Clone` to allow sharing
//! // them among multiple components.
//! fn metered_logic(metrics: MyMetrics) {
//!     // method logic...
//!     metrics.gauge.set(42);
//! }
//!
//! metered_logic(my_metrics);
//! ```

// Linter settings.
#![warn(missing_debug_implementations, missing_docs, bare_trait_objects)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::must_use_candidate, clippy::module_name_repetitions)]

pub use prometheus_client::{metrics::counter::Counter, registry::Unit};

/// Derives the [`EncodeLabelValue`] trait for a type, which encodes a metric label value.
///
/// The macro can be configured using `#[metrics()]` attributes.
///
/// # Container attributes
///
/// ## `format`
///
/// **Type:** string
///
/// **Default value:** `{}`
///
/// Specifies the format for the value as used in the `format!` macro etc. when encoding it to
/// a label value. For example, `{}` means using [`Display`](std::fmt::Display).
///
/// [`EncodeLabelValue`]: trait@prometheus_client::encoding::EncodeLabelValue
///
/// # Examples
///
///  ## Default format
///
/// Label value using the default `Display` formatting; note that `Display` itself is derived.
///
/// ```
/// use derive_more::Display;
/// use vise::EncodeLabelValue;
///
/// #[derive(Debug, Display, EncodeLabelValue)]
/// struct Method(&'static str);
/// ```
///
/// ## Custom format
///
/// Label value using `Hex` formatting with `0` padding and `0x` prepended.
///
/// ```
/// use derive_more::LowerHex;
/// use vise::EncodeLabelValue;
///
/// #[derive(Debug, LowerHex, EncodeLabelValue)]
/// #[metrics(format = "0x{:02x}")]
/// struct ResponseType(u8);
/// ```
pub use vise_macros::EncodeLabelValue;

/// Derives the [`EncodeLabelSet`] trait for a type, which encodes a set of metric labels.
///
/// The macro can be configured using `#[metrics()]` attributes.
///
/// # Container attributes
///
/// ## `label`
///
/// **Type:** string
///
/// If specified, the type will be treated as a single label with the given name. This covers
/// the common case in which a label set consists of a single label. In this case, the type
/// also needs to implement [`EncodeLabelValue`].
///
/// If this attribute is not specified (which is the default), a container must be a `struct`
/// with named fields. A label with the matching name will be created for each field.
///
/// # Field attributes
///
/// ## `skip`
///
/// **Type:** path to a function with `fn(&FieldType) -> bool` signature
///
/// This attribute works similarly to `skip_serializing_if` in `serde` – if the function it points
/// to returns `true` for the field value, the field will not be encoded as a label.
///
/// `Option` fields are skipped by default if they are `None` (i.e., they use `skip = Option::is_none`).
///
/// [`EncodeLabelSet`]: trait@prometheus_client::encoding::EncodeLabelSet
///
/// # Examples
///
/// ## Set with a single label
///
/// ```
/// use derive_more::Display;
/// use vise::{EncodeLabelSet, EncodeLabelValue};
///
/// #[derive(Debug, Display, Clone, PartialEq, Eq, Hash)]
/// #[derive(EncodeLabelValue, EncodeLabelSet)]
/// #[metrics(label = "method")]
/// struct Method(&'static str);
/// ```
///
/// ## Set with multiple labels
///
/// ```
/// # use vise::EncodeLabelSet;
/// #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelSet)]
/// struct Labels {
///     /// Label that is skipped when empty.
///     #[metrics(skip = str::is_empty)]
///     name: &'static str,
///     /// Numeric label.
///     num: u8,
/// }
/// ```
pub use vise_macros::EncodeLabelSet;

/// Derives the [`Metrics`](trait@Metrics) trait for a type.
///
/// This macro must be placed on a struct with named fields. Each field will be registered as metric
/// or a family of metrics. The macro can be configured using `#[metrics()]` attributes.
///
/// # Container attributes
///
/// ## `prefix`
///
/// **Type:** string
///
/// Specifies a common prefix for all metrics defined in the type. If specified, the prefix will
/// be prepended together with a `_` separator to a field name to get the metric name. Note that
/// the metric name may be additionally transformed depending on the unit and metric type.
///
/// # Field attributes
///
/// ## `buckets`
///
/// **Type:** expression evaluating to a type implementing `Into<`[`Buckets`]`>`
///
/// Specifies buckets for a [`Histogram`] or a [`Family`] of `Histogram`s. This attribute is mandatory
/// for these metric types and will result in a compile-time error if used on counters / gauges.
///
/// ## `unit`
///
/// **Type:** expression evaluating to [`Unit`]
///
/// Specifies unit of measurement for a metric. Note that specifying a unit influences the metric naming.
pub use vise_macros::Metrics;

/// Registers a [`Global`] metrics instance or [`Collector`], so that it will be included
/// into registries instantiated using [`Registry::collect()`].
///
/// This macro must be placed on a static item of a type implementing [`CollectToRegistry`].
///
/// # Examples
///
/// ## Usage with global metrics
///
/// ```
/// use vise::{Gauge, Global, Metrics};
///
/// #[derive(Debug, Metrics)]
/// #[metrics(prefix = "test")]
/// pub(crate) struct TestMetrics {
///     gauge: Gauge,
/// }
///
/// #[vise::register]
/// static TEST_METRICS: Global<TestMetrics> = Global::new();
/// ```
///
/// ## Usage with collectors
///
/// ```
/// use vise::{Collector, Gauge, Global, Metrics};
///
/// #[derive(Debug, Metrics)]
/// #[metrics(prefix = "dynamic")]
/// pub(crate) struct DynamicMetrics {
///     gauge: Gauge,
/// }
///
/// #[vise::register]
/// static TEST_COLLECTOR: Collector<DynamicMetrics> = Collector::new();
/// ```
pub use vise_macros::register;

#[doc(hidden)] // only used by the proc macros
pub mod _reexports {
    pub use linkme;
    pub use prometheus_client::{encoding, metrics::TypedMetric};
}

mod buckets;
mod collector;
mod constructor;
pub mod descriptors;
mod registry;
mod traits;
mod wrappers;

pub use crate::{
    buckets::Buckets,
    collector::Collector,
    constructor::{ConstructMetric, DefaultConstructor},
    registry::{MetricsVisitor, Registry, METRICS_REGISTRATIONS},
    traits::{CollectToRegistry, Global, Metrics},
    wrappers::{Family, Gauge, Histogram, LatencyObserver},
};

#[cfg(doctest)]
doc_comment::doctest!("../README.md");

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use assert_matches::assert_matches;
    use derive_more::Display;

    use std::time::Duration;

    use super::*;

    #[derive(Debug, Display, Clone, PartialEq, Eq, Hash, EncodeLabelValue, EncodeLabelSet)]
    #[metrics(crate = crate, label = "method")]
    struct Method(&'static str);

    impl From<&'static str> for Method {
        fn from(s: &'static str) -> Self {
            Self(s)
        }
    }

    #[derive(Debug, Metrics)]
    #[metrics(crate = crate, prefix = "test")]
    pub(crate) struct TestMetrics {
        /// Test counter.
        counter: Counter,
        #[metrics(unit = Unit::Bytes)]
        gauge: Gauge,
        /// Test family of gauges.
        family_of_gauges: Family<Method, Gauge<f64>>,
        /// Histogram with inline bucket specification.
        #[metrics(buckets = &[0.001, 0.002, 0.005, 0.01, 0.1])]
        histogram: Histogram<Duration>,
        /// A family of histograms with a multiline description.
        /// Note that we use a type alias to properly propagate bucket configuration.
        #[metrics(unit = Unit::Seconds, buckets = Buckets::LATENCIES)]
        family_of_histograms: Family<Method, Histogram<Duration>>,
        /// Family of histograms with a reference bucket specification.
        #[metrics(buckets = Buckets::ZERO_TO_ONE)]
        histograms_with_buckets: Family<Method, Histogram<Duration>>,
    }

    #[register]
    #[metrics(crate = crate)]
    static TEST_METRICS: Global<TestMetrics> = Global::new();

    #[test]
    fn metrics_registration() {
        let registry = Registry::collect();
        let descriptors = registry.descriptors();

        assert!(descriptors.metric_count() > 5);
        assert_eq!(descriptors.groups().len(), 2);
        // ^ We have `TestMetrics` above and `TestMetrics` in the `collectors` module
        assert!(descriptors
            .groups()
            .any(|group| group.module_path.contains("collector")));

        let counter_descriptor = descriptors.metric("test_counter").unwrap();
        assert_eq!(counter_descriptor.metric.help, "Test counter");

        // Test metric registered via a `Collector` in the corresponding module tests.
        let dynamic_gauge_descriptor = descriptors.metric("dynamic_gauge_bytes").unwrap();
        assert_matches!(dynamic_gauge_descriptor.metric.unit, Some(Unit::Bytes));
    }

    #[test]
    fn testing_metrics() {
        let test_metrics = &*TEST_METRICS;
        let mut registry = Registry::empty();
        registry.register_metrics(test_metrics);

        test_metrics.counter.inc();
        assert_eq!(test_metrics.counter.get(), 1);
        // ^ Counters and gauges can be easily tested

        test_metrics.gauge.set(42);
        assert_eq!(test_metrics.gauge.get(), 42);

        test_metrics.family_of_gauges[&"call".into()].set(0.4);
        test_metrics.family_of_gauges[&"send_transaction".into()].set(0.5);

        assert!(test_metrics.family_of_gauges.contains(&"call".into()));
        let gauge = test_metrics.family_of_gauges.get(&"call".into()).unwrap();
        assert_eq!(gauge.get(), 0.4);
        assert!(!test_metrics.family_of_gauges.contains(&"test".into()));

        let gauges_in_family = test_metrics.family_of_gauges.to_entries();
        assert_eq!(gauges_in_family.len(), 2);
        assert_eq!(gauges_in_family[&"call".into()].get(), 0.4);
        assert_eq!(gauges_in_family[&"send_transaction".into()].get(), 0.5);

        test_metrics.histogram.observe(Duration::from_millis(1));
        test_metrics.histogram.observe(Duration::from_micros(1_500));
        test_metrics.histogram.observe(Duration::from_millis(3));
        test_metrics.histogram.observe(Duration::from_millis(4));
        test_metrics.family_of_histograms[&"call".into()].observe(Duration::from_millis(20));

        test_metrics.histograms_with_buckets[&"call".into()].observe(Duration::from_millis(350));
        test_metrics.histograms_with_buckets[&"send_transaction".into()]
            .observe(Duration::from_millis(620));

        let mut buffer = String::new();
        registry.encode_to_text(&mut buffer).unwrap();
        let lines: Vec<_> = buffer.lines().collect();

        // `_bytes` suffix is added automatically per Prometheus naming suggestions:
        // https://prometheus.io/docs/practices/naming/
        assert!(lines.contains(&"# TYPE test_gauge_bytes gauge"));
        assert!(lines.contains(&"# UNIT test_gauge_bytes bytes"));
        assert!(lines.contains(&"test_gauge_bytes 42"));

        // Full stop is added to the metrics description automatically.
        assert!(lines.contains(&"# HELP test_family_of_gauges Test family of gauges."));
        assert!(lines.contains(&r#"test_family_of_gauges{method="call"} 0.4"#));
        assert!(lines.contains(&r#"test_family_of_gauges{method="send_transaction"} 0.5"#));

        let histogram_lines = [
            "test_histogram_sum 0.0095",
            "test_histogram_count 4",
            r#"test_histogram_bucket{le="0.001"} 1"#,
            r#"test_histogram_bucket{le="0.005"} 4"#,
            r#"test_histogram_bucket{le="0.01"} 4"#,
        ];
        for line in histogram_lines {
            assert!(
                lines.contains(&line),
                "text output doesn't contain line `{line}`"
            );
        }

        let long_description_line =
            "# HELP test_family_of_histograms_seconds A family of histograms \
            with a multiline description. Note that we use a type alias to properly propagate \
            bucket configuration.";
        assert!(lines.contains(&long_description_line));

        let histogram_family_lines = [
            r#"test_histograms_with_buckets_bucket{le="0.6",method="send_transaction"} 0"#,
            r#"test_histograms_with_buckets_bucket{le="0.7",method="send_transaction"} 1"#,
            r#"test_histograms_with_buckets_bucket{le="0.3",method="call"} 0"#,
            r#"test_histograms_with_buckets_bucket{le="0.4",method="call"} 1"#,
        ];
        for line in histogram_family_lines {
            assert!(
                lines.contains(&line),
                "text output doesn't contain line `{line}`"
            );
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelSet)]
    #[metrics(crate = crate)]
    struct Labels {
        /// Label that is skipped when empty.
        #[metrics(skip = str::is_empty)]
        name: &'static str,
        /// Label that is skipped when it's `None` (the default behavior).
        num: Option<u64>,
    }

    impl Labels {
        const fn named(name: &'static str) -> Self {
            Self { name, num: None }
        }

        const fn num(mut self, num: u64) -> Self {
            self.num = Some(num);
            self
        }
    }

    #[derive(Debug, Metrics)]
    #[metrics(crate = crate, prefix = "test")]
    struct MetricsWithLabels {
        /// Gauge with multiple labels.
        gauges: Family<Labels, Gauge<f64>>,
    }

    #[test]
    fn using_label_set() {
        let test_metrics = MetricsWithLabels::default();
        test_metrics.gauges[&Labels::named("test")].set(1.9);
        test_metrics.gauges[&Labels::named("test").num(5)].set(4.2);
        test_metrics.gauges[&Labels::named("").num(3)].set(2.0);

        let mut registry = Registry::empty();
        registry.register_metrics(&test_metrics);
        let mut buffer = String::new();
        registry.encode_to_text(&mut buffer).unwrap();
        let lines: Vec<_> = buffer.lines().collect();

        assert!(lines.contains(&r#"test_gauges{num="3"} 2.0"#));
        assert!(lines.contains(&r#"test_gauges{name="test"} 1.9"#));
        assert!(lines.contains(&r#"test_gauges{name="test",num="5"} 4.2"#));
    }
}
