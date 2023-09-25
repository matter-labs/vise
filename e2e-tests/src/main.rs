//! Mock app that defines `vise` metrics and uses the corresponding exporter.

use rand::{thread_rng, Rng};
use tokio::sync::watch;

use std::{env, time::Duration};

use vise::{
    Buckets, Counter, EncodeLabelSet, EncodeLabelValue, Family, Format, Gauge, Histogram,
    LabeledFamily, Metrics, Unit,
};
use vise_exporter::MetricsExporter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelValue, EncodeLabelSet)]
#[metrics(label = "method", rename_all = "snake_case")]
enum Method {
    Call,
    SendTransaction,
}

#[derive(Debug, Metrics)]
#[metrics(prefix = "test")]
struct TestMetrics {
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

impl TestMetrics {
    fn generate_metrics(&self, rng: &mut impl Rng) {
        self.counter.inc();
        self.gauge.set(rng.gen_range(0..1_000_000));
        self.family_of_gauges[&"call"].set(rng.gen_range(0.0..1.0));
        self.family_of_gauges[&"send_transaction"].set(rng.gen_range(0.0..1.0));

        for _ in 0..5 {
            self.histogram
                .observe(Duration::from_millis(rng.gen_range(0..100)));
            self.family_of_histograms[&Method::Call]
                .observe(Duration::from_micros(rng.gen_range(0..10_000)));
            self.family_of_histograms[&Method::SendTransaction]
                .observe(Duration::from_micros(rng.gen_range(0..20_000)));
            self.histograms_with_buckets[&"test"]
                .observe(Duration::from_millis(rng.gen_range(0..1_000)));
            self.histograms_with_buckets[&"other_test"]
                .observe(Duration::from_millis(rng.gen_range(0..1_000)));
        }
    }

    fn generate_legacy_metrics(rng: &mut impl Rng) {
        metrics::counter!("legacy_counter", 1);
        metrics::gauge!("legacy_gauge", rng.gen_range(0..1_000_000) as f64);
        metrics::gauge!(
            "legacy_family_of_gauges",
            rng.gen_range(0.0..1.0),
            "method" => "call"
        );
        metrics::gauge!(
            "legacy_family_of_gauges",
            rng.gen_range(0.0..1.0),
            "method" => "send_transaction"
        );

        for _ in 0..5 {
            metrics::histogram!(
                "legacy_histogram",
                Duration::from_millis(rng.gen_range(0..100))
            );
            metrics::histogram!(
                "legacy_family_of_histograms",
                Duration::from_micros(rng.gen_range(0..100)),
                "method" => "call"
            );
        }
    }
}

#[vise::register]
static METRICS: vise::Global<TestMetrics> = vise::Global::new();

#[tokio::main(flavor = "current_thread")]
async fn main() {
    const METRICS_INTERVAL: Duration = Duration::from_secs(5);

    let mut args: Vec<_> = env::args().skip(1).collect();
    let produce_legacy_metrics = if !args.is_empty() && args[0] == "--legacy" {
        args.remove(0);
        true
    } else {
        false
    };
    let export_format = if !args.is_empty() && args[0] == "--format-prometheus" {
        args.remove(0);
        Some(Format::Prometheus)
    } else {
        None
    };

    let bind_address = args
        .get(0)
        .expect("Bind address must be provided as first command-line arg");
    let bind_address = bind_address.parse().expect("Bind address is invalid");

    let (stop_sender, mut stop_receiver) = watch::channel(());
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        stop_sender.send_replace(());
    });

    let mut stop_receiver_copy = stop_receiver.clone();
    let mut exporter = MetricsExporter::default();
    if produce_legacy_metrics {
        exporter = exporter.with_legacy_exporter(|builder| {
            builder
                .set_buckets(&[0.001, 0.005, 0.025, 0.1, 0.25, 1.0, 5.0, 30.0, 120.0])
                .unwrap()
        });
    }
    if let Some(format) = export_format {
        exporter = exporter.with_format(format);
    }

    let exporter_server = exporter
        .with_graceful_shutdown(async move {
            stop_receiver_copy.changed().await.ok();
        })
        .bind(bind_address)
        .unwrap_or_else(|err| panic!("Failed binding to `{bind_address}`: {err}"));
    println!("local_addr={}", exporter_server.local_addr());
    // ^ Print the local server address so that it can be used in integration tests
    tokio::spawn(async {
        exporter_server.start().await.unwrap();
    });

    let mut rng = thread_rng();
    loop {
        METRICS.generate_metrics(&mut rng);
        if produce_legacy_metrics {
            TestMetrics::generate_legacy_metrics(&mut rng);
        }
        tokio::select! {
            _ = stop_receiver.changed() => break,
            () = tokio::time::sleep(METRICS_INTERVAL) => { /* continue looping */ }
        }
    }
}
