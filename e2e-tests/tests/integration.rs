//! Integration testing for `vise` exporter.

use anyhow::Context as _;
use assert_matches::assert_matches;
use prometheus_http_query::{response::MetricType, Client, TargetState};
use tokio::{
    fs,
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::Mutex,
};
use tracing::metadata::LevelFilter;

use std::{
    collections::HashSet,
    net::SocketAddr,
    path::Path,
    process::{Command as StdCommand, Stdio},
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

static DOCKER_RUN_MUTEX: Mutex<()> = Mutex::const_new(());

#[derive(Debug)]
struct PrometheusContainer {
    id: String,
}

impl PrometheusContainer {
    const PROM_IMAGE_TAG: &'static str = "v2.47.0";

    fn run_command(prom_config_path: &str) -> Command {
        let mut command = Command::new("docker");
        command
            .arg("run")
            // Resolve `host.docker.internal` to host IP (necessary for Linux)
            .args(["--add-host", "host.docker.internal:host-gateway"])
            // Volume for Prometheus config we've generated.
            .args([
                "-v",
                &format!("{prom_config_path}:/etc/prometheus/prometheus.yml"),
            ])
            .args(["-p", "0:9090"])
            .args(["--rm", "-d"])
            .arg(format!("prom/prometheus:{}", Self::PROM_IMAGE_TAG));

        let prom_args = [
            "--config.file=/etc/prometheus/prometheus.yml",
            "--web.console.libraries=/etc/prometheus/console_libraries",
            "--web.console.templates=/etc/prometheus/consoles",
            "--web.enable-lifecycle",
        ];
        command.args(prom_args);
        command
    }

    async fn new(temp_dir: &Path, app_port: u16) -> anyhow::Result<Self> {
        let prom_config = PROMETHEUS_CONFIG.replace("$port", &app_port.to_string());
        let prom_config_path = temp_dir.join("prometheus.yml");
        fs::write(&prom_config_path, prom_config)
            .await
            .context("Cannot write Prometheus config")?;
        let prom_config_path = prom_config_path
            .to_str()
            .context("Cannot convert path to Prometheus config")?;
        tracing::info!("Written Prometheus config to {prom_config_path}");

        let mut run_command = Self::run_command(prom_config_path);
        tracing::info!(?run_command, "Prepared run command for Docker");

        let run_lock = DOCKER_RUN_MUTEX.lock().await;
        let output = run_command.kill_on_drop(true).output().await?;
        drop(run_lock);

        assert!(output.status.success(), "`docker run` failed");
        let id = String::from_utf8(output.stdout)
            .context("Failed converting docker run stdout")?
            .trim()
            .to_owned();
        tracing::info!(id, "Started container");

        let this = Self { id };
        // Give a container some time to initialize.
        tokio::time::sleep(POLL_INTERVAL).await;
        this.wait_until_ready().await?;
        Ok(this)
    }

    async fn wait_until_ready(&self) -> anyhow::Result<()> {
        const READY_MESSAGE: &str = "Server is ready to receive web requests";

        let mut logs_process = Command::new("docker")
            .args(["logs", "-f", &self.id])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("Failed running `docker logs`")?;

        let stderr = logs_process.stderr.take().unwrap();
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        while let Some(line) = lines.next_line().await? {
            tracing::debug!(line, "Received log");
            if line.contains(READY_MESSAGE) {
                return Ok(());
            }
        }
        anyhow::bail!("Prometheus terminated without getting ready");
    }

    async fn port(&self) -> anyhow::Result<u16> {
        // Taken from https://docs.docker.com/engine/reference/commandline/inspect/
        const FORMAT: &str = "{{(index (index .NetworkSettings.Ports \"9090/tcp\") 0).HostPort}}";

        let output = Command::new("docker")
            .args(["inspect", "--type=container", "--format", FORMAT, &self.id])
            .arg(&self.id)
            .kill_on_drop(true)
            .output()
            .await?;
        let output = String::from_utf8(output.stdout)?;
        tracing::info!(output, "Received port");

        let ports: Result<HashSet<_>, _> = output
            .lines()
            .map(|line| line.trim().parse::<u16>())
            .collect();
        let ports = ports.context("Failed parsing Prometheus port")?;
        anyhow::ensure!(ports.len() == 1, "Ambiguous Prometheus port: {ports:?}");
        Ok(*ports.iter().next().unwrap())
    }
}

impl Drop for PrometheusContainer {
    fn drop(&mut self) {
        let output = StdCommand::new("docker")
            .args(["rm", "-f", "-v"])
            .arg(&self.id)
            .output()
            .expect("Failed running `docker rm`");
        assert!(output.status.success(), "`docker rm` failed");
        let output = String::from_utf8(output.stdout).expect("Failed decoding `docker rm` output");
        assert!(output.contains(&self.id), "Failed stopping container");
    }
}

fn init_logging() {
    tracing_subscriber::fmt()
        .pretty()
        .with_max_level(LevelFilter::INFO)
        .with_test_writer()
        .try_init()
        .ok();
}

async fn start_app(legacy_metrics: bool, prom_format: bool) -> anyhow::Result<(Child, u16)> {
    let binary = env!(concat!("CARGO_BIN_EXE_", env!("CARGO_PKG_NAME")));
    tracing::info!("Running binary `{binary}`");
    let mut command = Command::new(binary);
    if legacy_metrics {
        command.arg("--legacy");
    }
    if prom_format {
        command.arg("--format-prometheus");
    }
    let mut app_process = command
        .arg("0.0.0.0:0")
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("Failed spawning child")?;

    // The child should print its port to stdout.
    let app_stdout = app_process.stdout.take().unwrap();
    let app_stdout = BufReader::new(app_stdout);
    let line = app_stdout
        .lines()
        .next_line()
        .await?
        .context("app terminated prematurely")?;

    let app_addr = line
        .strip_prefix("local_addr=")
        .with_context(|| format!("Malformed app output: `{line}"))?
        .trim();
    let app_addr: SocketAddr = app_addr.parse()?;
    let app_port = app_addr.port();
    tracing::info!("Application started on {app_port}");

    Ok((app_process, app_port))
}

#[tracing::instrument(level = "info", err)]
async fn test_app(legacy_metrics: bool, prom_format: bool) -> anyhow::Result<()> {
    init_logging();

    let (_app_process, app_port) = start_app(legacy_metrics, prom_format).await?;

    let temp_dir = tempfile::tempdir().context("Failed creating temp dir")?;
    let container = PrometheusContainer::new(temp_dir.path(), app_port).await?;
    let prom_port = container.port().await?;
    tracing::info!("Prometheus started on {prom_port}");

    let client: Client = format!("http://localhost:{prom_port}/").parse()?;
    assert!(client.is_server_healthy().await.unwrap());
    assert_metrics(&client, prom_format).await?;
    if legacy_metrics {
        assert_legacy_metrics(&client).await?;
    }
    Ok(())
}

async fn assert_metrics(client: &Client, prom_format: bool) -> anyhow::Result<()> {
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
    let metadata = client
        .metric_metadata()
        .metric("test_package_metadata")
        .get()
        .await?;
    tracing::info!(?metadata, "Got metadata for info");
    let metadata = &metadata["test_package_metadata"][0];
    assert_eq!(metadata.help(), "Metadata about the current Cargo package.");
    if prom_format {
        assert_matches!(metadata.metric_type(), MetricType::Gauge);
    } else {
        assert_matches!(metadata.metric_type(), MetricType::Info);
    }

    let metadata = client
        .metric_metadata()
        .metric("test_counter")
        .get()
        .await?;
    tracing::info!(?metadata, "Got metadata for counter");
    let metadata = &metadata["test_counter"][0];
    assert_eq!(metadata.help(), "Test counter.");
    assert_matches!(metadata.metric_type(), MetricType::Counter);

    let metadata = client
        .metric_metadata()
        .metric("test_family_of_histograms_seconds")
        .get()
        .await?;
    tracing::info!(?metadata, "Got metadata for family of histograms");
    let metadata = &metadata["test_family_of_histograms_seconds"][0];
    let help = metadata.help();
    assert!(help.contains("family of histograms"), "{help}");
    assert!(help.contains("multiline description. Note that"), "{help}");
    assert_matches!(metadata.metric_type(), MetricType::Histogram);
    if !prom_format {
        // `# UNIT` declarations are ignored in the Prometheus format
        assert_eq!(metadata.unit(), "seconds");
    }

    let info_result = client.query("test_package_metadata").get().await?;
    tracing::info!(?info_result, "Got result for query: test_package_metadata");
    let info_vec = info_result
        .data()
        .as_vector()
        .context("Info data is not a vector")?;
    let info_labels = info_vec[0].metric();
    assert_eq!(info_labels["version"], "0.1.0", "{info_labels:?}");

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

async fn assert_legacy_metrics(client: &Client) -> anyhow::Result<()> {
    let metadata = client
        .metric_metadata()
        .metric("legacy_counter")
        .get()
        .await?;
    tracing::info!(?metadata, "Got metadata for legacy counter");
    let metadata = &metadata["legacy_counter"][0];
    assert_matches!(metadata.metric_type(), MetricType::Counter);

    let gauge_result = client.query("legacy_gauge").get().await?;
    tracing::info!(?gauge_result, "Got result for query: legacy_gauge");
    let gauge_vec = gauge_result
        .data()
        .as_vector()
        .context("Gauge data is not a vector")?;
    let gauge_value = gauge_vec[0].sample().value();
    assert!(
        (0.0..=1_000_000.0).contains(&gauge_value),
        "{gauge_result:#?}"
    );

    let family_result = client
        .query("legacy_family_of_gauges{method=\"call\"}")
        .get()
        .await?;
    tracing::info!(
        ?gauge_result,
        "Got result for query: legacy_family_of_gauges{{method=\"call\"}}"
    );
    let gauge_vec = family_result
        .data()
        .as_vector()
        .context("Gauge data is not a vector")?;
    let gauge_value = gauge_vec[0].sample().value();
    assert!((0.0..=1.0).contains(&gauge_value), "{family_result:#?}");
    assert_eq!(gauge_vec[0].metric()["method"], "call");

    Ok(())
}

#[tokio::test]
async fn scraping_app() -> anyhow::Result<()> {
    test_app(false, false).await
}

#[tokio::test]
async fn scraping_app_in_prom_format() -> anyhow::Result<()> {
    test_app(false, true).await
}

#[tokio::test]
async fn scraping_app_with_legacy_metrics() -> anyhow::Result<()> {
    test_app(true, false).await
}

#[tokio::test]
async fn scraping_app_with_legacy_metrics_and_prom_format() -> anyhow::Result<()> {
    test_app(true, true).await
}
