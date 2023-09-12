//! Metric exporter based on the `axum` web server.

// Linter settings.
#![warn(missing_debug_implementations, missing_docs, bare_trait_objects)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::must_use_candidate, clippy::module_name_repetitions)]

// Reexport to simplify configuring legacy exporter.
pub use metrics_exporter_prometheus;

use hyper::{
    body, header,
    service::{make_service_fn, service_fn},
    Body, Client, Method, Request, Response, Server, StatusCode, Uri,
};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use once_cell::sync::OnceCell;

use std::{
    borrow::Cow,
    collections::HashSet,
    fmt::{self, Write as _},
    future::{self, Future},
    net::SocketAddr,
    pin::Pin,
    str,
    sync::Arc,
    time::Duration,
};

mod metrics;

use crate::metrics::{Facade, EXPORTER_METRICS};
use vise::Registry;

static LEGACY_EXPORTER: OnceCell<PrometheusHandle> = OnceCell::new();

#[derive(Clone)]
struct MetricsExporterInner {
    registry: Arc<Registry>,
    legacy_exporter: Option<&'static PrometheusHandle>,
}

impl MetricsExporterInner {
    fn render_body(&self) -> Body {
        let latency = EXPORTER_METRICS.scrape_latency[&Facade::Metrics].start();
        let mut buffer = if let Some(legacy_exporter) = self.legacy_exporter {
            Self::transform_legacy_metrics(&legacy_exporter.render())
        } else {
            String::new()
        };

        let latency = latency.observe();
        let scraped_size = buffer.len();
        EXPORTER_METRICS.scraped_size[&Facade::Metrics].observe(scraped_size);
        tracing::debug!(
            latency_sec = latency.as_secs_f64(),
            scraped_size,
            "Scraped metrics using `metrics` façade in {latency:?} (scraped size: {scraped_size}B)"
        );

        let latency = EXPORTER_METRICS.scrape_latency[&Facade::Vise].start();
        let mut new_buffer = String::with_capacity(1_024);
        self.registry.encode_to_text(&mut new_buffer).unwrap();
        let new_buffer = Self::transform_new_metrics(&new_buffer);
        // ^ `unwrap()` is safe; writing to a string never fails.

        let latency = latency.observe();
        let scraped_size = new_buffer.len();
        EXPORTER_METRICS.scraped_size[&Facade::Vise].observe(scraped_size);
        tracing::debug!(
            latency_sec = latency.as_secs_f64(),
            scraped_size,
            "Scraped metrics using `vise` façade in {latency:?} (scraped size: {scraped_size}B)"
        );

        // Concatenate buffers. Since `legacy_buffer` ends with a newline (if it isn't empty),
        // we don't need to add a newline.
        buffer.push_str(&new_buffer);
        Body::from(buffer)
    }

    /// Transforms legacy metrics from the Prometheus text format to the Open Metrics one.
    ///
    /// This transform:
    ///
    /// - Removes empty lines from `buffer`; they are fine for Prometheus, but run contrary
    ///   to the Open Metrics text format spec.
    fn transform_legacy_metrics(buffer: &str) -> String {
        buffer
            .lines()
            .filter(|line| !line.is_empty())
            .flat_map(|line| [line, "\n"])
            .collect()
    }

    /// Transforms the Open Metrics text format so that it can be properly ingested by Prometheus.
    ///
    /// Prometheus *mostly* understands the Open Metrics format (e.g., enforcing no empty lines for it;
    /// see the transform above). The notable exception is counter definitions; Open Metrics requires
    /// to append `_total` to the counter name (like `_sum` / `_count` / `_bucket` are appended
    /// to histogram names), but Prometheus doesn't understand this (yet?).
    ///
    /// See also: [issue in `prometheus-client`](https://github.com/prometheus/client_rust/issues/111)
    ///
    /// This transform:
    ///
    /// - Strips `_total` suffix from counter definitions.
    fn transform_new_metrics(buffer: &str) -> String {
        let mut last_metric_type = None;

        buffer
            .lines()
            .flat_map(|line| {
                let mut transformed_line = None;
                if let Some(type_def) = line.strip_prefix("# TYPE ") {
                    last_metric_type = Some(MetricTypeDef::parse(type_def));
                } else if !line.starts_with('#') {
                    // `line` reports metric value
                    let name_end_pos = line
                        .find(|ch: char| ch == '{' || ch.is_ascii_whitespace())
                        .unwrap_or_else(|| {
                            panic!("Invalid metric definition: {line}");
                        });
                    let (name, rest) = line.split_at(name_end_pos);

                    if let Some(metric_type) = last_metric_type {
                        let truncated_name = name.strip_suffix("_total");

                        if truncated_name == Some(metric_type.name) && metric_type.is_counter() {
                            // Remove `_total` suffix to the metric name, which is not present
                            // in the Prometheus text format, but is mandatory for the Open Metrics format.
                            transformed_line = Some(format!("{}{rest}", metric_type.name));
                        }
                    }
                }

                let transformed_line = transformed_line.map_or(Cow::Borrowed(line), Cow::Owned);
                [transformed_line, Cow::Borrowed("\n")]
            }) // restore newlines
            .collect()
    }

    // TODO: consider using a streaming response?
    fn render(&self) -> Response<Body> {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, TEXT_CONTENT_TYPE)
            .body(self.render_body())
            .unwrap()
    }
}

#[derive(Debug, Clone, Copy)]
struct MetricTypeDef<'a> {
    name: &'a str,
    ty: &'a str,
}

impl<'a> MetricTypeDef<'a> {
    fn parse(raw: &'a str) -> Self {
        let (name, ty) = raw
            .trim()
            .split_once(|ch: char| ch.is_ascii_whitespace())
            .unwrap_or_else(|| {
                panic!("Invalid metric type definition: {raw}");
            });
        Self { name, ty }
    }

    fn is_counter(self) -> bool {
        self.ty == "counter"
    }
}

/// Metrics exporter to Prometheus.
pub struct MetricsExporter {
    inner: MetricsExporterInner,
    shutdown_future: Pin<Box<dyn Future<Output = ()> + Send>>,
}

impl fmt::Debug for MetricsExporter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetricsExporter")
            .field("registry", &self.inner.registry)
            .finish_non_exhaustive()
    }
}

/// Creates an exporter based on [`Registry::collect()`] output (i.e., with all metrics registered
/// by the app and libs it depends on).
impl Default for MetricsExporter {
    fn default() -> Self {
        Self::new(Registry::collect().into())
    }
}

impl MetricsExporter {
    /// Creates an exporter based on the provided metrics [`Registry`]. Note that the registry
    /// is in `Arc`, meaning it can be used elsewhere (e.g., to export data in another format).
    pub fn new(registry: Arc<Registry>) -> Self {
        Self::log_metrics_stats(&registry);
        Self {
            inner: MetricsExporterInner {
                registry,
                legacy_exporter: None,
            },
            shutdown_future: Box::pin(future::pending()),
        }
    }

    fn log_metrics_stats(registry: &Registry) {
        const SAMPLED_CRATE_COUNT: usize = 5;

        let groups = registry.descriptors().groups();
        let group_count = groups.len();
        let metric_count = registry.descriptors().metric_count();

        let mut unique_crates = HashSet::new();
        for group in groups {
            let crate_info = (group.crate_name, group.crate_version);
            if unique_crates.insert(crate_info) && unique_crates.len() >= SAMPLED_CRATE_COUNT {
                break;
            }
        }
        let mut crates = String::with_capacity(unique_crates.len() * 16);
        // ^ 16 chars looks like a somewhat reasonable estimate for crate name + version
        for (crate_name, crate_version) in unique_crates {
            write!(crates, "{crate_name} {crate_version}, ").unwrap();
        }
        crates.push_str("...");

        tracing::info!(
            "Created metrics exporter with {metric_count} metrics in {group_count} groups from crates {crates}"
        );
    }

    /// Installs a legacy exporter for the metrics defined using the `metrics` façade. The specified
    /// closure allows customizing the exporter, e.g. specifying buckets for histograms.
    ///
    /// The exporter can only be installed once during app lifetime, so if it was installed previously,
    /// the same instance will be reused, and the closure won't be called.
    ///
    /// # Panics
    ///
    /// If `exporter_fn` panics, it is propagated to the caller.
    #[must_use]
    pub fn with_legacy_exporter<F>(mut self, exporter_fn: F) -> Self
    where
        F: FnOnce(PrometheusBuilder) -> PrometheusBuilder,
    {
        let legacy_exporter = LEGACY_EXPORTER
            .get_or_try_init(|| {
                let builder = exporter_fn(PrometheusBuilder::new());
                builder.install_recorder()
            })
            .expect("Failed installing recorder for `metrics` façade");

        self.inner.legacy_exporter = Some(legacy_exporter);
        self
    }

    /// Configures graceful shutdown for the exporter server.
    #[must_use]
    pub fn with_graceful_shutdown<F>(mut self, shutdown: F) -> Self
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.shutdown_future = Box::pin(shutdown);
        self
    }

    /// Starts the server on the specified address. This future resolves when the server is shut down.
    ///
    /// The server will expose the following endpoints:
    ///
    /// - `GET` on any path: serves the metrics in the Open Metrics text format
    ///
    /// # Panics
    ///
    /// Panics if binding to the specified address fails.
    pub async fn start(self, bind_address: SocketAddr) {
        tracing::info!("Starting Prometheus exporter web server on {bind_address}");

        Server::bind(&bind_address)
            .serve(make_service_fn(move |_| {
                let inner = self.inner.clone();
                future::ready(Ok::<_, hyper::Error>(service_fn(move |_| {
                    let inner = inner.clone();
                    async move { Ok::<_, hyper::Error>(inner.render()) }
                })))
            }))
            .with_graceful_shutdown(async move {
                self.shutdown_future.await;
                tracing::info!(
                    "Stop signal received, Prometheus metrics exporter is shutting down"
                );
            })
            .await
            .expect("Metrics server failed to start");

        tracing::info!("Prometheus metrics exporter server shut down");
    }

    /// Starts pushing metrics to the `endpoint` with the specified `interval` between pushes.
    #[allow(clippy::missing_panics_doc)]
    pub async fn push_to_gateway(self, endpoint: Uri, interval: Duration) {
        tracing::info!(
            "Starting push-based Prometheus exporter to `{endpoint}` with push interval {interval:?}"
        );

        let client = Client::new();
        let mut shutdown = self.shutdown_future;
        loop {
            if tokio::time::timeout(interval, &mut shutdown).await.is_ok() {
                tracing::info!(
                    "Stop signal received, Prometheus metrics exporter is shutting down"
                );
                break;
            }

            let request = Request::builder()
                .method(Method::PUT)
                .uri(endpoint.clone())
                .header(header::CONTENT_TYPE, TEXT_CONTENT_TYPE)
                .body(self.inner.render_body())
                .expect("Failed creating Prometheus push gateway request");

            match client.request(request).await {
                Ok(response) => {
                    if !response.status().is_success() {
                        // Do not block further pushes during error handling.
                        tokio::spawn(Self::report_erroneous_response(response));
                    }
                }
                Err(err) => {
                    tracing::error!(%err, "Error submitting metrics to Prometheus push gateway");
                }
            }
        }
    }

    async fn report_erroneous_response(response: Response<Body>) {
        let status = response.status();
        let body = match body::to_bytes(response.into_body()).await {
            Ok(body) => body,
            Err(err) => {
                tracing::error!(
                    %err,
                    %status,
                    "Failed reading erroneous response from Prometheus push gateway"
                );
                return;
            }
        };

        let err_body: String;
        let body = match str::from_utf8(&body) {
            Ok(body) => body,
            Err(err) => {
                let body_length = body.len();
                err_body = format!("(Non UTF-8 body with length {body_length}B: {err})");
                &err_body
            }
        };
        tracing::warn!(
            %status,
            %body,
            "Error pushing metrics to Prometheus push gateway"
        );
    }
}

const TEXT_CONTENT_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

#[cfg(doctest)]
doc_comment::doctest!("../README.md");

#[cfg(test)]
mod tests {
    use ::metrics;
    use hyper::body::Bytes;
    use tokio::sync::{mpsc, Mutex};

    use std::{
        net::Ipv4Addr,
        str,
        sync::atomic::{AtomicU32, Ordering},
    };

    use super::*;
    use vise::{Counter, EncodeLabelSet, EncodeLabelValue, Family, Gauge, Global, Metrics};

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

    #[test]
    fn transforming_open_metrics_text_format() {
        let input = "\
            # TYPE modern_counter counter\n\
            modern_counter_total 1\n\
            # TYPE modern_gauge gauge\n\
            modern_gauge{label=\"value\"} 23\n\
            # TYPE modern_counter_with_labels counter\n\
            modern_counter_with_labels_total{label=\"value\"} 3\n\
            modern_counter_with_labels_total{label=\"other\"} 5";
        let expected = "\
            # TYPE modern_counter counter\n\
            modern_counter 1\n\
            # TYPE modern_gauge gauge\n\
            modern_gauge{label=\"value\"} 23\n\
            # TYPE modern_counter_with_labels counter\n\
            modern_counter_with_labels{label=\"value\"} 3\n\
            modern_counter_with_labels{label=\"other\"} 5\n";

        let transformed = MetricsExporterInner::transform_new_metrics(input);
        assert_eq!(transformed, expected);
    }

    fn init_legacy_exporter(builder: PrometheusBuilder) -> PrometheusBuilder {
        let default_buckets = [0.001, 0.005, 0.025, 0.1, 0.25, 1.0, 5.0, 30.0, 120.0];
        builder.set_buckets(&default_buckets).unwrap()
    }

    #[tokio::test]
    async fn legacy_and_modern_metrics_can_coexist() {
        let _guard = TEST_MUTEX.lock().await;
        let exporter = MetricsExporter::new(Registry::collect().into())
            .with_legacy_exporter(init_legacy_exporter);
        report_metrics();

        let response = exporter.inner.render();
        let response = body::to_bytes(response.into_body()).await;
        let response = response.expect("failed decoding response");
        assert_scraped_payload_is_valid(&response);
    }

    fn report_metrics() {
        TEST_METRICS.counter.inc();
        TEST_METRICS.gauge[&Label("value")].set(42.0);
        metrics::increment_counter!("legacy_counter");
        metrics::increment_counter!("legacy_counter_with_labels", "label" => "value", "code" => "3");
        metrics::gauge!("legacy_gauge", 23.0, "label" => "value");
    }

    fn assert_scraped_payload_is_valid(payload: &Bytes) {
        let payload = str::from_utf8(payload).unwrap();
        let payload_lines: Vec<_> = payload.lines().collect();

        assert!(payload_lines.iter().all(|line| !line.is_empty()));

        let expected_lines = [
            "# TYPE modern_counter counter",
            "# TYPE legacy_counter counter",
            "# TYPE legacy_counter_with_labels counter",
            "# TYPE legacy_gauge gauge",
            r#"legacy_gauge{label="value"} 23"#,
            "# TYPE modern_gauge gauge",
            r#"modern_gauge{label="value"} 42.0"#,
        ];
        for line in expected_lines {
            assert!(payload_lines.contains(&line), "{payload_lines:#?}");
        }

        // Check counter reporting.
        let expected_prefixes = [
            "modern_counter ",
            "legacy_counter ",
            r#"legacy_counter_with_labels{label="value",code="3"} "#,
        ];
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

        fn response(self) -> Response<Body> {
            match self {
                Self::Ok => Response::builder()
                    .status(StatusCode::ACCEPTED)
                    .body(Body::empty())
                    .unwrap(),
                Self::Error => Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .body(Body::empty())
                    .unwrap(),
                Self::Panic => panic!("oops"),
            }
        }
    }

    #[tokio::test]
    async fn using_push_gateway() {
        static REQUEST_COUNTER: AtomicU32 = AtomicU32::new(0);

        let _guard = TEST_MUTEX.lock().await;
        let bind_address: SocketAddr = (Ipv4Addr::LOCALHOST, 0).into();
        let (req_sender, mut req_receiver) = mpsc::unbounded_channel();

        // Bind the mock server to a random free port.
        let mock_server = Server::bind(&bind_address).serve(make_service_fn(move |_| {
            let req_sender = req_sender.clone();
            future::ready(Ok::<_, hyper::Error>(service_fn(move |req| {
                assert_eq!(*req.method(), Method::PUT);

                let behavior = MockServerBehavior::from_counter(&REQUEST_COUNTER);
                let req_sender = req_sender.clone();
                async move {
                    let headers = req.headers().clone();
                    let body = body::to_bytes(req.into_body()).await?;
                    req_sender.send((headers, body)).ok();
                    Ok::<_, hyper::Error>(behavior.response())
                }
            })))
        }));
        let local_addr = mock_server.local_addr();
        tokio::spawn(mock_server);

        let exporter = MetricsExporter::new(Registry::collect().into())
            .with_legacy_exporter(init_legacy_exporter);
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
            assert_eq!(request_headers[&header::CONTENT_TYPE], TEXT_CONTENT_TYPE);
            assert_scraped_payload_is_valid(&request_body);
        }
    }
}
