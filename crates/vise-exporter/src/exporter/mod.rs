//! `MetricsExporter` and closely related types.

use std::{
    collections::HashSet,
    fmt::{self, Write as _},
    future::{self, Future},
    net::SocketAddr,
    pin::Pin,
    str,
    sync::Arc,
    time::{Duration, Instant},
};

use hyper::{
    body, header,
    service::{make_service_fn, service_fn},
    Body, Client, Method, Request, Response, Server, StatusCode, Uri,
};
#[cfg(feature = "legacy")]
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

#[cfg(test)]
mod tests;

use vise::{Format, MetricsCollection, Registry};

use crate::metrics::{Facade, EXPORTER_METRICS};

#[derive(Clone)]
struct MetricsExporterInner {
    registry: Arc<Registry>,
    format: Format,
    #[cfg(feature = "legacy")]
    legacy_exporter: Option<&'static PrometheusHandle>,
}

impl MetricsExporterInner {
    async fn render_body(&self) -> Body {
        let mut buffer = self.scrape_legacy_metrics();

        let latency = EXPORTER_METRICS.scrape_latency[&Facade::Vise].start();
        let registry = Arc::clone(&self.registry);
        let format = self.format;
        // `Registry::encode()` is blocking in the general case (specifically, if collectors are used; they may use
        // blocking I/O etc.). We cannot make metric collection non-blocking because the underlying library only provides
        // blocking interface for collectors.
        let new_buffer = tokio::task::spawn_blocking(move || {
            let mut new_buffer = String::with_capacity(1_024);
            registry.encode(&mut new_buffer, format).unwrap();
            // ^ `unwrap()` is safe; writing to a string never fails.
            new_buffer
        })
        .await
        .unwrap(); // propagate panics should they occur in the spawned blocking task

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

    #[cfg(feature = "legacy")]
    fn scrape_legacy_metrics(&self) -> String {
        let latency = EXPORTER_METRICS.scrape_latency[&Facade::Metrics].start();
        let buffer = if let Some(legacy_exporter) = self.legacy_exporter {
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
        buffer
    }

    #[cfg(not(feature = "legacy"))]
    #[allow(clippy::unused_self)] // required for consistency with the real method
    fn scrape_legacy_metrics(&self) -> String {
        String::new()
    }

    /// Transforms legacy metrics from the Prometheus text format to the OpenMetrics one.
    /// The output format is still accepted by Prometheus.
    ///
    /// This transform:
    ///
    /// - Removes empty lines from `buffer`; they are fine for Prometheus, but run contrary
    ///   to the OpenMetrics text format spec.
    #[cfg(feature = "legacy")]
    fn transform_legacy_metrics(buffer: &str) -> String {
        buffer
            .lines()
            .filter(|line| !line.is_empty())
            .flat_map(|line| [line, "\n"])
            .collect()
    }

    async fn render(&self) -> Response<Body> {
        let content_type = if matches!(self.format, Format::Prometheus) {
            Format::PROMETHEUS_CONTENT_TYPE
        } else {
            Format::OPEN_METRICS_CONTENT_TYPE
        };
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type)
            .body(self.render_body().await)
            .unwrap()
    }
}

/// Metrics exporter to Prometheus.
///
/// An exporter scrapes metrics from a [`Registry`]. A [`Default`] exporter will use the registry
/// of all metrics auto-registered in an app and all its (transitive) dependencies, i.e. one
/// created using [`Registry::collect()`]. To have more granular control over the registry, you can
/// provide it explicitly using [`Self::new()`].
///
/// # Examples
///
/// See crate-level docs for the examples of usage.
pub struct MetricsExporter<'a> {
    inner: MetricsExporterInner,
    shutdown_future: Pin<Box<dyn Future<Output = ()> + Send + 'a>>,
}

impl fmt::Debug for MetricsExporter<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetricsExporter")
            .field("registry", &self.inner.registry)
            .finish_non_exhaustive()
    }
}

/// Creates an exporter based on [`MetricsCollection`]`::default().collect()` output (i.e., with all metrics
/// registered by the app and libs it depends on).
impl Default for MetricsExporter<'_> {
    fn default() -> Self {
        Self::new(MetricsCollection::default().collect().into())
    }
}

impl<'a> MetricsExporter<'a> {
    /// Creates an exporter based on the provided metrics [`Registry`]. Note that the registry
    /// is in `Arc`, meaning it can be used elsewhere (e.g., to export data in another format).
    pub fn new(registry: Arc<Registry>) -> Self {
        Self::log_metrics_stats(&registry);
        Self {
            inner: MetricsExporterInner {
                registry,
                format: Format::OpenMetricsForPrometheus,
                #[cfg(feature = "legacy")]
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

    /// Sets the export [`Format`]. By default, [`Format::OpenMetricsForPrometheus`] is used
    /// (i.e., OpenMetrics text format with minor changes so that it is fully parsed by Prometheus).
    ///
    /// See `Format` docs for more details on differences between export formats. Note that using
    /// [`Format::OpenMetrics`] is not fully supported by Prometheus at the time of writing.
    #[must_use]
    pub fn with_format(mut self, format: Format) -> Self {
        self.inner.format = format;
        self
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
    #[cfg(feature = "legacy")]
    #[cfg_attr(docsrs, doc(cfg(feature = "legacy")))]
    pub fn with_legacy_exporter<F>(mut self, exporter_fn: F) -> Self
    where
        F: FnOnce(PrometheusBuilder) -> PrometheusBuilder,
    {
        use once_cell::sync::OnceCell;

        static LEGACY_EXPORTER: OnceCell<PrometheusHandle> = OnceCell::new();

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
        F: Future<Output = ()> + Send + 'a,
    {
        self.shutdown_future = Box::pin(shutdown);
        self
    }

    /// Starts the server on the specified address. This future resolves when the server is shut down.
    ///
    /// The server will expose the following endpoints:
    ///
    /// - `GET` on any path: serves the metrics in the text format configured using [`Self::with_format()`]
    ///
    /// # Errors
    ///
    /// Returns an error if binding to the specified address fails.
    pub async fn start(self, bind_address: SocketAddr) -> hyper::Result<()> {
        tracing::info!("Starting Prometheus exporter web server on {bind_address}");
        self.bind(bind_address)?.start().await?;
        tracing::info!("Prometheus metrics exporter server shut down");
        Ok(())
    }

    /// Creates an HTTP exporter server and binds it to the specified address.
    ///
    /// # Errors
    ///
    /// Returns an error if binding to the specified address fails.
    pub fn bind(self, bind_address: SocketAddr) -> hyper::Result<MetricsServer<'a>> {
        let server = Server::try_bind(&bind_address)?.serve(make_service_fn(move |_| {
            let inner = self.inner.clone();
            future::ready(Ok::<_, hyper::Error>(service_fn(move |_| {
                let inner = inner.clone();
                async move { Ok::<_, hyper::Error>(inner.render().await) }
            })))
        }));
        let local_addr = server.local_addr();

        let server = server.with_graceful_shutdown(async move {
            self.shutdown_future.await;
            tracing::info!("Stop signal received, Prometheus metrics exporter is shutting down");
        });
        Ok(MetricsServer {
            server: Box::pin(server),
            local_addr,
        })
    }

    /// Starts pushing metrics to the `endpoint` with the specified `interval` between pushes.
    #[allow(clippy::missing_panics_doc)]
    pub async fn push_to_gateway(self, endpoint: Uri, interval: Duration) {
        /// Minimum interval between error logs. Prevents spanning logs at `WARN` / `ERROR` level
        /// too frequently if `interval` is low (e.g., 1s).
        const ERROR_LOG_INTERVAL: Duration = Duration::from_secs(60);

        tracing::info!(
            "Starting push-based Prometheus exporter to `{endpoint}` with push interval {interval:?}"
        );

        let client = Client::new();
        let mut shutdown = self.shutdown_future;
        let mut last_error_log_timestamp = None::<Instant>;
        loop {
            let mut shutdown_requested = false;
            if tokio::time::timeout(interval, &mut shutdown).await.is_ok() {
                tracing::info!(
                    "Stop signal received, Prometheus metrics exporter is shutting down"
                );
                shutdown_requested = true;
            }

            let request = Request::builder()
                .method(Method::PUT)
                .uri(endpoint.clone())
                .header(header::CONTENT_TYPE, Format::OPEN_METRICS_CONTENT_TYPE)
                .body(self.inner.render_body().await)
                .expect("Failed creating Prometheus push gateway request");

            match client.request(request).await {
                Ok(response) => {
                    if !response.status().is_success() {
                        let should_log_error = last_error_log_timestamp
                            .map_or(true, |timestamp| timestamp.elapsed() >= ERROR_LOG_INTERVAL);
                        if should_log_error {
                            // Do not block further pushes during error handling.
                            tokio::spawn(report_erroneous_response(endpoint.clone(), response));
                            last_error_log_timestamp = Some(Instant::now());
                            // ^ This timestamp is somewhat imprecise (we don't wait to handle the response),
                            // but it seems fine for rate-limiting purposes.
                        }
                    }
                }
                Err(err) => {
                    let should_log_error = last_error_log_timestamp
                        .map_or(true, |timestamp| timestamp.elapsed() >= ERROR_LOG_INTERVAL);
                    if should_log_error {
                        tracing::error!(
                            %err,
                            %endpoint,
                            "Error submitting metrics to Prometheus push gateway"
                        );
                        last_error_log_timestamp = Some(Instant::now());
                    }
                }
            }
            if shutdown_requested {
                break;
            }
        }
    }
}

async fn report_erroneous_response(endpoint: Uri, response: Response<Body>) {
    let status = response.status();
    let body = match body::to_bytes(response.into_body()).await {
        Ok(body) => body,
        Err(err) => {
            tracing::error!(
                %err,
                %status,
                %endpoint,
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
        %endpoint,
        "Error pushing metrics to Prometheus push gateway"
    );
}

/// Metrics server bound to a certain local address returned by [`MetricsExporter::bind()`].
///
/// Useful e.g. if you need to find out which port the server was bound to if the 0th port was specified.
#[must_use = "Server should be `start()`ed"]
pub struct MetricsServer<'a> {
    server: Pin<Box<dyn Future<Output = hyper::Result<()>> + Send + 'a>>,
    local_addr: SocketAddr,
}

impl fmt::Debug for MetricsServer<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetricsServer")
            .field("local_addr", &self.local_addr)
            .finish_non_exhaustive()
    }
}

impl MetricsServer<'_> {
    /// Returns the local address this server is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Starts this server.
    ///
    /// # Errors
    ///
    /// Returns an error if starting the server operation fails.
    pub async fn start(self) -> hyper::Result<()> {
        self.server.await
    }
}
