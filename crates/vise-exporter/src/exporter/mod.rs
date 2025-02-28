//! `MetricsExporter` and closely related types.

use std::{
    collections::HashSet,
    convert::Infallible,
    fmt::{self, Write as _},
    future::{self, Future},
    net::SocketAddr,
    pin::Pin,
    str,
    sync::Arc,
    time::{Duration, Instant},
};

use http_body_util::BodyExt as _;
use hyper::{
    body::Incoming, header, server::conn::http1, service::service_fn, Method, Request, Response,
    StatusCode, Uri,
};
use hyper_util::{
    client::legacy::Client,
    rt::{TokioExecutor, TokioIo},
};
use tokio::{io, net::TcpListener, sync::watch};
use vise::{Format, MetricsCollection, Registry};

use crate::metrics::{Facade, EXPORTER_METRICS};

#[cfg(test)]
mod tests;

#[derive(Clone)]
struct MetricsExporterInner {
    registry: Arc<Registry>,
    format: Format,
}

impl MetricsExporterInner {
    async fn render_body(&self) -> String {
        let mut buffer = String::new();

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
        buffer
    }

    async fn render(&self) -> Response<String> {
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
    pub async fn start(self, bind_address: SocketAddr) -> io::Result<()> {
        tracing::info!("Starting Prometheus exporter web server on {bind_address}");
        self.bind(bind_address).await?.start().await?;
        tracing::info!("Prometheus metrics exporter server shut down");
        Ok(())
    }

    /// Creates an HTTP exporter server and binds it to the specified address.
    ///
    /// # Errors
    ///
    /// Returns an error if binding to the specified address fails.
    pub async fn bind(mut self, bind_address: SocketAddr) -> io::Result<MetricsServer<'a>> {
        let listener = TcpListener::bind(bind_address).await?;
        let local_addr = listener.local_addr()?;
        let server = async move {
            let (started_shutdown_sender, started_shutdown) = watch::channel(());
            loop {
                let stream = tokio::select! {
                    res = listener.accept() => res?.0,
                    () = &mut self.shutdown_future => break,
                };

                let io = TokioIo::new(stream);
                let inner = self.inner.clone();
                let mut started_shutdown = started_shutdown.clone();
                tokio::spawn(async move {
                    let conn = http1::Builder::new().serve_connection(
                        io,
                        service_fn(|_| async { Ok::<_, Infallible>(inner.render().await) }),
                    );
                    tokio::pin!(conn);

                    let res = tokio::select! {
                        _ = started_shutdown.changed() => {
                            conn.as_mut().graceful_shutdown();
                            conn.await
                        }
                        res = conn.as_mut() => res,
                    };
                    if let Err(err) = res {
                        tracing::warn!(%err, "Error serving connection");
                    }
                });
            }

            tracing::info!("Stop signal received, Prometheus metrics exporter is shutting down");
            // Send the graceful shutdown signal to all alive connections.
            drop(started_shutdown);
            started_shutdown_sender.send_replace(());
            // Wait until all connections are dropped.
            started_shutdown_sender.closed().await;

            Ok(())
        };

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

        let client = Client::builder(TokioExecutor::new()).build_http();
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

async fn report_erroneous_response(endpoint: Uri, response: Response<Incoming>) {
    let status = response.status();

    let body = match response.into_body().collect().await {
        Ok(body) => body.to_bytes(),
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
    server: Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>>,
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

    /// Starts this server. Resolves once the server is shut down.
    ///
    /// # Errors
    ///
    /// Returns an error if starting the server operation fails.
    pub async fn start(self) -> io::Result<()> {
        self.server.await
    }
}
