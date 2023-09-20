#![allow(clippy::float_cmp)]

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
    gauge: Gauge<usize>,
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

    let long_description_line = "# HELP test_family_of_histograms_seconds A family of histograms \
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

#[test]
fn label_with_raw_ident() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelSet)]
    #[metrics(crate = crate)]
    struct LabelWithRawIdent {
        r#type: &'static str,
    }

    impl From<&'static str> for LabelWithRawIdent {
        fn from(r#type: &'static str) -> Self {
            Self { r#type }
        }
    }

    #[derive(Debug, Metrics)]
    #[metrics(crate = crate, prefix = "test")]
    struct MetricsWithLabels {
        counters: Family<LabelWithRawIdent, Counter>,
    }

    let test_metrics = MetricsWithLabels::default();
    test_metrics.counters[&"first".into()].inc();

    let mut registry = Registry::empty();
    registry.register_metrics(&test_metrics);
    let mut buffer = String::new();
    registry.encode_to_text(&mut buffer).unwrap();
    let lines: Vec<_> = buffer.lines().collect();

    assert!(
        lines.contains(&r#"test_counters_total{type="first"} 1"#),
        "{lines:#?}"
    );
}

#[test]
fn renamed_labels() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelValue, EncodeLabelSet)]
    #[metrics(crate = crate, rename_all = "snake_case", label = "kind")]
    enum KindLabel {
        First,
        #[metrics(name = "2nd")]
        Second,
        ThirdOrMore,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelValue, EncodeLabelSet)]
    #[metrics(crate = crate, rename_all = "SCREAMING-KEBAB-CASE", label = "kind")]
    enum ScreamingLabel {
        Postgres,
        MySql,
    }

    #[derive(Debug, Metrics)]
    #[metrics(crate = crate, prefix = "test")]
    struct MetricsWithLabels {
        counters: Family<KindLabel, Counter>,
        gauges: Family<ScreamingLabel, Gauge>,
    }

    let test_metrics = MetricsWithLabels::default();
    test_metrics.counters[&KindLabel::First].inc();
    test_metrics.counters[&KindLabel::Second].inc_by(23);
    test_metrics.counters[&KindLabel::ThirdOrMore].inc_by(42);
    test_metrics.gauges[&ScreamingLabel::Postgres].set(5);
    test_metrics.gauges[&ScreamingLabel::MySql].set(3);

    let mut registry = Registry::empty();
    registry.register_metrics(&test_metrics);
    let mut buffer = String::new();
    registry.encode_to_text(&mut buffer).unwrap();
    let lines: Vec<_> = buffer.lines().collect();

    assert!(
        lines.contains(&r#"test_counters_total{kind="first"} 1"#),
        "{lines:#?}"
    );
    assert!(
        lines.contains(&r#"test_counters_total{kind="2nd"} 23"#),
        "{lines:#?}"
    );
    assert!(
        lines.contains(&r#"test_counters_total{kind="third_or_more"} 42"#),
        "{lines:#?}"
    );
    assert!(
        lines.contains(&r#"test_gauges{kind="POSTGRES"} 5"#),
        "{lines:#?}"
    );
    assert!(
        lines.contains(&r#"test_gauges{kind="MY-SQL"} 3"#),
        "{lines:#?}"
    );
}
