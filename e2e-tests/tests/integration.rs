//! Integration testing for `vise` exporter.

use anyhow::Context as _;
use assert_matches::assert_matches;
use prometheus_http_query::{response::MetricType, Client, TargetState};
use testcontainers::{clients::Cli, core::WaitFor, images::generic::GenericImage, RunnableImage};
use tracing::metadata::LevelFilter;

use std::{
    collections::HashSet,
    fs,
    io::{BufRead, BufReader},
    net::SocketAddr,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const PROMETHEUS_CONFIG: &str = r#"
global:
  scrape_interval: 1s

scrape_configs:
  - job_name: 'prometheus'
    scrape_interval: 1s
    static_configs:
      - targets: ['localhost:9090']
  - job_name: 'app'
    scrape_interval: 1s
    static_configs:
      - targets: [ 'host.docker.internal:$port' ]
"#;

const POLL_INTERVAL: Duration = Duration::from_millis(250);
const MAX_WAIT: Duration = Duration::from_secs(20);

#[derive(Debug)]
struct ChildGuard(Child);

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self(child)
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.0.kill().ok();
    }
}

fn prometheus_image(temp_dir: &Path, app_port: u16) -> anyhow::Result<RunnableImage<GenericImage>> {
    const PROM_IMAGE_TAG: &str = "v2.47.0";
    const READY_MESSAGE: &str = "Server is ready to receive web requests";

    let prom_config = PROMETHEUS_CONFIG.replace("$port", &app_port.to_string());
    let prom_config_path = temp_dir.join("prometheus.yml");
    fs::write(&prom_config_path, prom_config).context("Cannot write Prometheus config")?;
    let prom_config_path = prom_config_path
        .to_str()
        .context("Cannot convert path to Prometheus config")?;
    tracing::info!("Written Prometheus config to {prom_config_path}");

    let image = GenericImage::new("prom/prometheus", PROM_IMAGE_TAG)
        .with_exposed_port(9090)
        .with_wait_for(WaitFor::message_on_stderr(READY_MESSAGE));
    let args = [
        "--config.file=/etc/prometheus/prometheus.yml",
        "--web.console.libraries=/etc/prometheus/console_libraries",
        "--web.console.templates=/etc/prometheus/consoles",
        "--web.enable-lifecycle",
    ];
    let args: Vec<_> = args.into_iter().map(str::to_owned).collect();
    Ok(RunnableImage::from((image, args))
        .with_volume((prom_config_path, "/etc/prometheus/prometheus.yml"))
        .with_mapped_port((0, 9090)))
}

fn init_logging() {
    tracing_subscriber::fmt()
        .pretty()
        .with_max_level(LevelFilter::INFO)
        .with_test_writer()
        .init();
}

fn start_app() -> anyhow::Result<(ChildGuard, u16)> {
    let binary = env!(concat!("CARGO_BIN_EXE_", env!("CARGO_PKG_NAME")));
    tracing::info!("Running binary `{binary}`");
    let app_process = Command::new(binary)
        .arg("127.0.0.1:0")
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed spawning child")?;
    let mut app_process = ChildGuard::new(app_process);

    // The child should print its port to stdout.
    let app_stdout = app_process.0.stdout.take().context("no app stdout")?;
    let mut app_stdout = BufReader::new(app_stdout);
    let mut line = String::new();
    app_stdout.read_line(&mut line)?;

    let app_addr = line
        .strip_prefix("local_addr=")
        .with_context(|| format!("Malformed app output: `{line}"))?
        .trim();
    let app_addr: SocketAddr = app_addr.parse()?;
    let app_port = app_addr.port();
    tracing::info!("Application started on {app_port}");

    Ok((app_process, app_port))
}

#[tokio::test]
async fn starting_and_scraping_app() -> anyhow::Result<()> {
    init_logging();

    let (_app_process, app_port) = tokio::task::spawn_blocking(start_app).await??;

    let cli = Cli::docker();
    let temp_dir = tempfile::tempdir().context("Failed creating temp dir")?;
    let container = cli.run(prometheus_image(temp_dir.path(), app_port)?);

    let prom_port = container
        .ports()
        .map_to_host_port_ipv4(9090)
        .context("Prometheus container doesn't map port 9090")?;
    tracing::info!("Prometheus started on port {prom_port}");

    assert_metrics(prom_port).await
}

async fn assert_metrics(prom_port: u16) -> anyhow::Result<()> {
    let client: Client = format!("http://localhost:{prom_port}/").parse()?;
    assert!(client.is_server_healthy().await.unwrap());

    // Wait until the app is scraped.
    let started_at = Instant::now();
    loop {
        tokio::time::sleep(POLL_INTERVAL).await;
        if started_at.elapsed() > MAX_WAIT {
            anyhow::bail!("Timed out waiting for app to be scraped");
        }

        let result = client.targets(Some(TargetState::Active)).await?;
        tracing::info!(?result, "Got targets");
        let app_target = result
            .active()
            .iter()
            .find(|target| target.labels()["job"] == "app");
        let Some(app_target) = app_target else {
            continue;
        };
        if app_target.health().to_string() == "up" {
            break;
        }
    }

    // Check metrics metadata.
    let metadata = client.metric_metadata(Some("test_counter"), None).await?;
    tracing::info!(?metadata, "Got metadata for counter");
    let metadata = &metadata["test_counter"][0];
    assert_eq!(metadata.help(), "Test counter.");
    assert_matches!(metadata.metric_type(), MetricType::Counter);

    let metadata = client
        .metric_metadata(Some("test_family_of_histograms_seconds"), None)
        .await?;
    tracing::info!(?metadata, "Got metadata for family of histograms");
    let metadata = &metadata["test_family_of_histograms_seconds"][0];
    let help = metadata.help();
    assert!(help.contains("family of histograms"), "{help}");
    assert!(help.contains("multiline description. Note that"), "{help}");
    assert_matches!(metadata.metric_type(), MetricType::Histogram);
    assert_eq!(metadata.unit(), "seconds");

    let gauge_result = client.query("test_gauge_bytes").get().await?;
    tracing::info!(?gauge_result, "Got result for query: test_gauge_bytes");
    let gauge_vec = gauge_result
        .data()
        .as_vector()
        .context("Gauge data is not a vector")?;
    let gauge_value = gauge_vec[0].sample().value();
    assert!(
        (0.0..=1_000_000.0).contains(&gauge_value),
        "{gauge_result:#?}"
    );
    // ^ Bounds here and below correspond to `TestMetrics::generate_metrics()`

    let family_result = client
        .query("test_family_of_gauges{method=\"call\"}")
        .get()
        .await?;
    tracing::info!(
        ?gauge_result,
        "Got result for query: test_family_of_gauges{{method=\"call\"}}"
    );
    let gauge_vec = family_result
        .data()
        .as_vector()
        .context("Gauge data is not a vector")?;
    let gauge_value = gauge_vec[0].sample().value();
    assert!((0.0..=1.0).contains(&gauge_value), "{family_result:#?}");
    assert_eq!(gauge_vec[0].metric()["method"], "call");

    let family_result = client
        .query("test_family_of_histograms_seconds_sum")
        .get()
        .await?;
    tracing::info!(
        ?family_result,
        "Got result for query: test_family_of_histograms_seconds_sum"
    );
    let histogram_vec = family_result
        .data()
        .as_vector()
        .context("Histogram data is not a vector")?;
    assert_eq!(histogram_vec.len(), 2);
    let labels = histogram_vec
        .iter()
        .map(|iv| iv.metric()["method"].as_str());
    let labels: HashSet<_> = labels.collect();
    assert_eq!(labels, HashSet::from(["call", "send_transaction"]));

    Ok(())
}
