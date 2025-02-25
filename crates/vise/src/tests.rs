#![allow(clippy::float_cmp)]

use std::time::Duration;

use assert_matches::assert_matches;
use derive_more::Display;

use super::*;

#[derive(Debug, Display, Clone, PartialEq, Eq, Hash, EncodeLabelValue, EncodeLabelSet)]
#[metrics(crate = crate, label = "method")]
struct Method(&'static str);

impl From<&'static str> for Method {
    fn from(s: &'static str) -> Self {
        Self(s)
    }
}

#[derive(Debug, PartialEq, EncodeLabelSet)]
#[metrics(crate = crate)]
struct PackageMetadata {
    version: &'static str,
}

#[derive(Debug, Metrics)]
#[metrics(crate = crate, prefix = "test")]
pub(crate) struct TestMetrics {
    /// Test information.
    package_metadata: Info<PackageMetadata>,
    /// Test counter.
    counter: Counter,
    #[metrics(unit = Unit::Bytes)]
    gauge: Gauge<usize>,
    /// Test family of gauges.
    #[metrics(labels = ["method"])]
    family_of_gauges: LabeledFamily<&'static str, Gauge<f64>>,
    /// Histogram with inline bucket specification.
    #[metrics(buckets = &[0.001, 0.002, 0.005, 0.01, 0.1])]
    histogram: Histogram<Duration>,
    /// A family of histograms with a multiline description.
    /// Note that we use a type alias to properly propagate bucket configuration.
    #[metrics(unit = Unit::Seconds, buckets = Buckets::LATENCIES)]
    family_of_histograms: Family<Method, Histogram<Duration>>,
    /// Family of histograms with a reference bucket specification.
    #[metrics(buckets = Buckets::ZERO_TO_ONE, labels = ["method"])]
    histograms_with_buckets: LabeledFamily<&'static str, Histogram<Duration>>,
}

#[test]
fn testing_metrics() {
    let test_metrics = TestMetrics::default();
    let mut registry = Registry::empty();
    registry.register_metrics(&test_metrics);

    assert_eq!(test_metrics.package_metadata.get(), None);
    test_metrics
        .package_metadata
        .set(PackageMetadata { version: "0.1.0" })
        .unwrap();
    assert_eq!(
        test_metrics.package_metadata.get(),
        Some(&PackageMetadata { version: "0.1.0" })
    );

    test_metrics.counter.inc();
    assert_eq!(test_metrics.counter.get(), 1);
    // ^ Counters and gauges can be easily tested

    test_metrics.gauge.set(42);
    assert_eq!(test_metrics.gauge.get(), 42);

    test_metrics.family_of_gauges[&"call"].set(0.4);
    test_metrics.family_of_gauges[&"send_transaction"].set(0.5);

    assert!(test_metrics.family_of_gauges.contains(&"call"));
    let gauge = test_metrics.family_of_gauges.get(&"call").unwrap();
    assert_eq!(gauge.get(), 0.4);
    assert!(!test_metrics.family_of_gauges.contains(&"test"));

    let gauges_in_family = test_metrics.family_of_gauges.to_entries();
    assert_eq!(gauges_in_family.len(), 2);
    assert_eq!(gauges_in_family[&"call"].get(), 0.4);
    assert_eq!(gauges_in_family[&"send_transaction"].get(), 0.5);

    test_metrics.histogram.observe(Duration::from_millis(1));
    test_metrics.histogram.observe(Duration::from_micros(1_500));
    test_metrics.histogram.observe(Duration::from_millis(3));
    test_metrics.histogram.observe(Duration::from_millis(4));
    test_metrics.family_of_histograms[&"call".into()].observe(Duration::from_millis(20));

    test_metrics.histograms_with_buckets[&"call"].observe(Duration::from_millis(350));
    test_metrics.histograms_with_buckets[&"send_transaction"].observe(Duration::from_millis(620));

    let mut buffer = String::new();
    registry.encode(&mut buffer, Format::OpenMetrics).unwrap();
    let lines: Vec<_> = buffer.lines().collect();

    assert!(lines.contains(&"# TYPE test_package_metadata info"));
    assert!(lines.contains(&"# HELP test_package_metadata Test information."));
    assert!(lines.contains(&r#"test_package_metadata_info{version="0.1.0"} 1"#));

    // `_bytes` suffix is added automatically per Prometheus naming suggestions:
    // https://prometheus.io/docs/practices/naming/
    assert!(lines.contains(&"# TYPE test_gauge_bytes gauge"));
    assert!(lines.contains(&"# UNIT test_gauge_bytes bytes"));
    assert!(lines.contains(&"test_gauge_bytes 42"));

    // Full stop is added to the metrics description automatically.
    assert!(lines.contains(&"# HELP test_family_of_gauges Test family of gauges."));
    assert!(
        lines.contains(&r#"test_family_of_gauges{method="call"} 0.4"#),
        "{lines:#?}"
    );
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

#[test]
fn metrics_registration() {
    #[register]
    #[metrics(crate = crate)]
    static TEST_METRICS: Global<TestMetrics> = Global::new();

    let registry = MetricsCollection::default()
        .filter(|group| group.name == "TestMetrics")
        .collect();
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

    TEST_METRICS.counter.inc_by(3);
    TEST_METRICS.histogram.observe(Duration::from_millis(5));
    assert_test_metrics(&registry);
}

fn assert_test_metrics(registry: &Registry) {
    let mut buffer = String::new();
    registry.encode(&mut buffer, Format::OpenMetrics).unwrap();
    let lines: Vec<_> = buffer.lines().collect();

    let expected_lines = [
        "# TYPE test_counter counter",
        "# HELP test_counter Test counter.",
        "test_counter_total 3",
        "# TYPE test_histogram histogram",
        "test_histogram_bucket{le=\"0.01\"} 1",
    ];
    for expected_line in expected_lines {
        assert!(
            lines.contains(&expected_line),
            "Missing line `{expected_line}`: {lines:#?}"
        );
    }
}

#[test]
fn group_registration() {
    #[derive(Debug, Metrics)]
    #[metrics(crate = crate, prefix = "rpc_method")]
    struct MethodMetrics {
        #[metrics(labels = ["code"])]
        return_codes: LabeledFamily<i16, Counter>,
        errors: Counter,
        #[metrics(buckets = Buckets::LATENCIES)]
        latency: Histogram<Duration>,
    }

    #[register]
    #[metrics(crate = crate)]
    static GROUP_METRICS: MetricsFamily<Method, MethodMetrics> = MetricsFamily::new();

    let registry = MetricsCollection::default()
        .filter(|group| group.name == "MethodMetrics")
        .collect();

    GROUP_METRICS[&Method("eth_call")]
        .latency
        .observe(Duration::from_millis(100));
    GROUP_METRICS[&Method("eth_call")].errors.inc();
    GROUP_METRICS[&Method("eth_call")].return_codes[&0].inc_by(5);
    GROUP_METRICS[&Method("eth_call")].return_codes[&3].inc_by(2);
    GROUP_METRICS[&Method("eth_call")].return_codes[&-2].inc();
    GROUP_METRICS[&Method("eth_blockNumber")]
        .latency
        .observe(Duration::from_millis(200));
    GROUP_METRICS[&Method("eth_blockNumber")].return_codes[&0].inc_by(7);

    let mut buffer = String::new();
    registry.encode(&mut buffer, Format::OpenMetrics).unwrap();
    let lines: Vec<_> = buffer.lines().collect();

    // Each metric should be defined exactly once.
    for metric_name in [
        "rpc_method_errors",
        "rpc_method_latency",
        "rpc_method_return_codes",
    ] {
        let type_definition_start = format!("# TYPE {metric_name} ");
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.starts_with(&type_definition_start))
                .count(),
            1
        );
    }

    let expected_lines = [
        "rpc_method_return_codes_total{method=\"eth_call\",code=\"0\"} 5",
        "rpc_method_return_codes_total{method=\"eth_call\",code=\"-2\"} 1",
        "rpc_method_return_codes_total{method=\"eth_call\",code=\"3\"} 2",
        "rpc_method_return_codes_total{method=\"eth_blockNumber\",code=\"0\"} 7",
        "rpc_method_errors_total{method=\"eth_call\"} 1",
        "rpc_method_latency_sum{method=\"eth_call\"} 0.1",
        "rpc_method_latency_count{method=\"eth_call\"} 1",
        "rpc_method_latency_sum{method=\"eth_blockNumber\"} 0.2",
        "rpc_method_latency_count{method=\"eth_blockNumber\"} 1",
    ];
    for expected_line in expected_lines {
        assert!(lines.contains(&expected_line), "{lines:#?}");
    }
}

#[test]
fn lazy_metrics_registration() {
    // Metric names clash with `TestMetrics` above, but because we use group filtering in both cases,
    // this is fine.
    #[derive(Debug, Metrics)]
    #[metrics(crate = crate, prefix = "test")]
    pub(crate) struct LazyMetrics {
        /// Test counter.
        counter: Counter,
        /// Histogram with inline bucket specification.
        #[metrics(buckets = &[0.001, 0.002, 0.005, 0.01, 0.1])]
        histogram: Histogram<Duration>,
    }

    #[register]
    #[metrics(crate = crate)]
    static LAZY_METRICS: Global<LazyMetrics> = Global::new();

    let registry = MetricsCollection::lazy()
        .filter(|group| group.name == "LazyMetrics")
        .collect();
    let descriptors = registry.descriptors();

    assert_eq!(descriptors.metric_count(), 2);
    assert_eq!(descriptors.groups().len(), 1);
    let counter_descriptor = descriptors.metric("test_counter").unwrap();
    assert_eq!(counter_descriptor.metric.help, "Test counter");

    let mut buffer = String::new();
    registry.encode(&mut buffer, Format::OpenMetrics).unwrap();
    assert_eq!(buffer, "# EOF\n");

    LAZY_METRICS.counter.inc_by(3);
    LAZY_METRICS.histogram.observe(Duration::from_millis(5));
    assert_test_metrics(&registry);
}

#[test]
fn using_gauge_guard() {
    let test_metrics: TestMetrics = TestMetrics::default();
    let guard = test_metrics.gauge.inc_guard(5);
    let other_guard = test_metrics.gauge.inc_guard(3);
    assert_eq!(test_metrics.gauge.get(), 8);
    drop(other_guard);
    assert_eq!(test_metrics.gauge.get(), 5);
    drop(guard);
    assert_eq!(test_metrics.gauge.get(), 0);
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
    registry.encode(&mut buffer, Format::OpenMetrics).unwrap();
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
    registry.encode(&mut buffer, Format::OpenMetrics).unwrap();
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
    registry.encode(&mut buffer, Format::OpenMetrics).unwrap();
    let lines: Vec<_> = buffer.lines().collect();

    let expected_lines = [
        r#"test_counters_total{kind="first"} 1"#,
        r#"test_counters_total{kind="2nd"} 23"#,
        r#"test_counters_total{kind="third_or_more"} 42"#,
        r#"test_gauges{kind="POSTGRES"} 5"#,
        r#"test_gauges{kind="MY-SQL"} 3"#,
    ];
    for line in expected_lines {
        assert!(lines.contains(&line), "{lines:#?}");
    }
}

#[test]
fn labels_with_unit() {
    #[derive(Debug, EncodeLabelSet)]
    #[metrics(crate = crate)]
    struct LabelsWithUnits {
        #[metrics(unit = Unit::Bytes)]
        capacity: u64,
        #[metrics(unit = Unit::Seconds)]
        timeout: f64,
    }

    #[derive(Debug, Metrics)]
    #[metrics(crate = crate, prefix = "test")]
    struct InfoMetrics {
        config: Info<LabelsWithUnits>,
    }

    let test_metrics = InfoMetrics::default();
    test_metrics
        .config
        .set(LabelsWithUnits {
            capacity: 128,
            timeout: 0.1,
        })
        .ok();

    let mut registry = Registry::empty();
    registry.register_metrics(&test_metrics);
    let mut buffer = String::new();
    registry.encode(&mut buffer, Format::OpenMetrics).unwrap();
    let lines: Vec<_> = buffer.lines().collect();

    assert!(
        lines.contains(&r#"test_config_info{capacity_bytes="128",timeout_seconds="0.1"} 1"#),
        "{lines:#?}"
    );
}

#[test]
fn labeled_family_with_multiple_labels() {
    type ThreeLabels = (&'static str, &'static str, u8);
    const LABEL_NAMES: [&str; 3] = ["db", "cf", "code"];

    #[derive(Debug, Metrics)]
    #[metrics(crate = crate, prefix = "test")]
    struct MetricsWithLabels {
        /// Counters labeled by a tuple with the corresponding 2 label names specified via an attribute.
        #[metrics(labels = ["method", "code"])]
        counters: LabeledFamily<(&'static str, u16), Counter, 2>,
        #[metrics(labels = LABEL_NAMES)]
        gauges: LabeledFamily<ThreeLabels, Gauge<f64>, 3>,
    }

    let test_metrics = MetricsWithLabels::default();
    test_metrics.counters[&("call", 200)].inc_by(10);
    test_metrics.counters[&("call", 400)].inc();
    test_metrics.counters[&("send_transaction", 200)].inc_by(8);
    test_metrics.counters[&("send_transaction", 502)].inc_by(3);
    test_metrics.gauges[&("tree", "default", 0)].set(42.0);
    test_metrics.gauges[&("tree", "default", 1)].set(23.0);
    test_metrics.gauges[&("tree", "stale_keys", 0)].set(20.0);

    let mut registry = Registry::empty();
    registry.register_metrics(&test_metrics);
    let mut buffer = String::new();
    registry.encode(&mut buffer, Format::OpenMetrics).unwrap();
    let lines: Vec<_> = buffer.lines().collect();

    let expected_lines = [
        "test_counters_total{method=\"call\",code=\"400\"} 1",
        "test_counters_total{method=\"send_transaction\",code=\"502\"} 3",
        "test_counters_total{method=\"send_transaction\",code=\"200\"} 8",
        "test_counters_total{method=\"call\",code=\"200\"} 10",
        "test_gauges{db=\"tree\",cf=\"default\",code=\"0\"} 42.0",
        "test_gauges{db=\"tree\",cf=\"default\",code=\"1\"} 23.0",
        "test_gauges{db=\"tree\",cf=\"stale_keys\",code=\"0\"} 20.0",
    ];
    for line in expected_lines {
        assert!(lines.contains(&line), "{lines:#?}");
    }
}
