//! Tests for metrics exporter.

use std::{
    net::Ipv4Addr,
    str,
    sync::atomic::{AtomicU32, Ordering},
};

use tokio::sync::{mpsc, Mutex};
use tracing::subscriber::Subscriber;
use tracing_capture::{CaptureLayer, SharedStorage};
use tracing_subscriber::layer::SubscriberExt;
use vise::{Counter, EncodeLabelSet, EncodeLabelValue, Family, Gauge, Global, Metrics};

use super::*;

const TEST_TIMEOUT: Duration = Duration::from_secs(3);
// Since all tests access global state (metrics), we shouldn't run them in parallel
static TEST_MUTEX: Mutex<()> = Mutex::const_new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelValue, EncodeLabelSet)]
#[metrics(label = "label")]
struct Label(&'static str);

impl fmt::Display for Label {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Debug, Metrics)]
#[metrics(prefix = "modern")]
struct TestMetrics {
    /// Counter.
    counter: Counter,
    /// Gauge with a label defined using the modern approach.
    gauge: Family<Label, Gauge<f64>>,
}

#[vise::register]
static TEST_METRICS: Global<TestMetrics> = Global::new();

#[cfg(feature = "legacy")]
fn init_legacy_exporter(builder: PrometheusBuilder) -> PrometheusBuilder {
    let default_buckets = [0.001, 0.005, 0.025, 0.1, 0.25, 1.0, 5.0, 30.0, 120.0];
    builder.set_buckets(&default_buckets).unwrap()
}

#[tokio::test]
async fn legacy_and_modern_metrics_can_coexist() {
    let _guard = TEST_MUTEX.lock().await;
    let exporter = MetricsExporter::default();
    #[cfg(feature = "legacy")]
    let exporter = exporter.with_legacy_exporter(init_legacy_exporter);
    report_metrics();

    let response = exporter.inner.render().await.into_body();
    assert_scraped_payload_is_valid(&response);
}

fn report_metrics() {
    TEST_METRICS.counter.inc();
    TEST_METRICS.gauge[&Label("value")].set(42.0);

    #[cfg(feature = "legacy")]
    {
        metrics::increment_counter!("legacy_counter");
        metrics::increment_counter!(
            "legacy_counter_with_labels",
            "label" => "value",
            "code" => "3"
        );
        metrics::gauge!("legacy_gauge", 23.0, "label" => "value");
    }
}

fn assert_scraped_payload_is_valid(payload: &str) {
    let payload_lines: Vec<_> = payload.lines().collect();

    assert!(payload_lines.iter().all(|line| !line.is_empty()));

    let expected_lines = [
        "# TYPE modern_counter counter",
        "# TYPE modern_gauge gauge",
        r#"modern_gauge{label="value"} 42.0"#,
    ];
    #[cfg(feature = "legacy")]
    let expected_lines = expected_lines.into_iter().chain([
        "# TYPE legacy_counter counter",
        "# TYPE legacy_counter_with_labels counter",
        "# TYPE legacy_gauge gauge",
        r#"legacy_gauge{label="value"} 23"#,
    ]);
    for line in expected_lines {
        assert!(payload_lines.contains(&line), "{payload_lines:#?}");
    }

    // Check counter reporting.
    let expected_prefixes = ["modern_counter "];
    #[cfg(feature = "legacy")]
    let expected_prefixes = expected_prefixes.into_iter().chain([
        "legacy_counter ",
        r#"legacy_counter_with_labels{label="value",code="3"} "#,
    ]);
    for prefix in expected_prefixes {
        assert!(
            payload_lines.iter().any(|line| line.starts_with(prefix)),
            "{payload_lines:#?}"
        );
    }

    let lines_count = payload_lines.len();
    assert_eq!(*payload_lines.last().unwrap(), "# EOF");
    for &line in &payload_lines[..lines_count - 1] {
        assert_ne!(line, "# EOF");
    }
}

#[derive(Debug)]
enum MockServerBehavior {
    Ok,
    Error,
    Panic,
}

impl MockServerBehavior {
    fn from_counter(counter: &AtomicU32) -> Self {
        match counter.fetch_add(1, Ordering::SeqCst) % 3 {
            1 => Self::Error,
            2 => Self::Panic,
            _ => Self::Ok,
        }
    }

    fn response(self) -> Response<String> {
        match self {
            Self::Ok => Response::builder()
                .status(StatusCode::ACCEPTED)
                .body(String::new())
                .unwrap(),
            Self::Error => Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .body("Mistake!".into())
                .unwrap(),
            Self::Panic => panic!("oops"),
        }
    }
}

fn tracing_subscriber(storage: &SharedStorage) -> impl Subscriber {
    tracing_subscriber::fmt()
        .pretty()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .finish()
        .with(CaptureLayer::new(storage))
}

#[tokio::test]
async fn graceful_shutdown_works_as_expected() {
    let (shutdown_sender, mut shutdown) = watch::channel(());
    let exporter = MetricsExporter::default().with_graceful_shutdown(async move {
        shutdown.changed().await.ok();
    });
    #[cfg(feature = "legacy")]
    let exporter = exporter.with_legacy_exporter(init_legacy_exporter);

    let bind_address: SocketAddr = (Ipv4Addr::LOCALHOST, 0).into();
    let server = exporter.bind(bind_address).await.unwrap();
    let local_addr = server.local_addr();
    let server_task = tokio::spawn(server.start());
    report_metrics();

    let url: Uri = format!("http://{local_addr}/metrics").parse().unwrap();
    let client = Client::builder(TokioExecutor::new()).build_http::<String>();
    let response = client.get(url.clone()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload = response.into_body().collect().await.unwrap().to_bytes();
    let payload = str::from_utf8(&payload).unwrap();
    assert_scraped_payload_is_valid(payload);

    shutdown_sender.send_replace(());
    server_task.await.unwrap().unwrap();

    let err = client.get(url).await.unwrap_err();
    assert!(err.is_connect(), "{err:?}");
}

#[tokio::test]
async fn using_push_gateway() {
    static REQUEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    let _guard = TEST_MUTEX.lock().await;
    let tracing_storage = SharedStorage::default();
    let _subscriber_guard = tracing::subscriber::set_default(tracing_subscriber(&tracing_storage));
    // ^ **NB.** `set_default()` only works because tests use a single-threaded Tokio runtime

    let bind_address: SocketAddr = (Ipv4Addr::LOCALHOST, 0).into();
    let (req_sender, mut req_receiver) = mpsc::unbounded_channel();

    // Bind the mock server to a random free port.
    let listener = TcpListener::bind(bind_address).await.unwrap();
    let local_addr = listener.local_addr().unwrap();
    let service = service_fn(move |req: Request<Incoming>| {
        assert_eq!(*req.method(), Method::PUT);

        let behavior = MockServerBehavior::from_counter(&REQUEST_COUNTER);
        let req_sender = req_sender.clone();
        async move {
            let headers = req.headers().clone();
            let body = req.into_body().collect().await?.to_bytes();
            req_sender.send((headers, body)).ok();
            Ok::<_, hyper::Error>(behavior.response())
        }
    });

    tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(socket);
            let service = service.clone();
            tokio::spawn(async move {
                http1::Builder::new()
                    .serve_connection(io, service)
                    .await
                    .unwrap();
            });
        }
    });

    let exporter = MetricsExporter::default();
    #[cfg(feature = "legacy")]
    let exporter = exporter.with_legacy_exporter(init_legacy_exporter);
    report_metrics();

    let endpoint = format!("http://{local_addr}/").parse().unwrap();
    tokio::spawn(exporter.push_to_gateway(endpoint, Duration::from_millis(50)));

    // Test that the push logic doesn't stop after an error received from the gateway
    for _ in 0..4 {
        let (request_headers, request_body) =
            tokio::time::timeout(TEST_TIMEOUT, req_receiver.recv())
                .await
                .expect("timed out waiting for metrics push")
                .unwrap();
        assert_eq!(
            request_headers[&header::CONTENT_TYPE],
            Format::OPEN_METRICS_CONTENT_TYPE
        );
        let request_body = str::from_utf8(&request_body).unwrap();
        assert_scraped_payload_is_valid(request_body);
    }

    assert_logs(&tracing_storage.lock());
}

fn assert_logs(tracing_storage: &tracing_capture::Storage) {
    let warnings = tracing_storage.all_events().filter(|event| {
        event
            .metadata()
            .target()
            .starts_with(env!("CARGO_CRATE_NAME"))
            && *event.metadata().level() <= tracing::Level::WARN
    });
    let warnings: Vec<_> = warnings.collect();
    // Check that we don't spam the error messages.
    assert_eq!(warnings.len(), 1);

    // Check warning contents. We should log the first encountered error (i.e., "Service unavailable").
    let warning: &tracing_capture::CapturedEvent = &warnings[0];
    assert!(warning
        .message()
        .unwrap()
        .contains("Error pushing metrics to Prometheus push gateway"));
    assert_eq!(
        warning["status"].as_debug_str().unwrap(),
        StatusCode::SERVICE_UNAVAILABLE.to_string()
    );
    assert_eq!(warning["body"].as_debug_str().unwrap(), "Mistake!");
    assert!(warning["endpoint"]
        .as_debug_str()
        .unwrap()
        .starts_with("http://127.0.0.1:"));
}
