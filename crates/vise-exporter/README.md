# Metrics Exporter for `vise`

This crate provides a simple [Prometheus] metrics exporter for metrics defined
using [`vise`]. It is based on the [`hyper`] library and supports both pull-based
and push-based communication with Prometheus.

## Usage

An exporter can be initialized from a metrics `Registry`:

```rust
use tokio::sync::watch;

use vise::Registry;
use vise_exporter::MetricsExporter;

async fn my_app() {
    let registry = Registry::collect();
    let (shutdown_sender, mut shutdown_receiver) = watch::channel(());
    let exporter = MetricsExporter::new(registry.into())
        .with_graceful_shutdown(async move {
            shutdown_receiver.changed().await.ok();
        });
    let bind_address = "0.0.0.0:3312".parse().unwrap();
    tokio::spawn(exporter.start(bind_address));

    // Then, once the app is shutting down:
    shutdown_sender.send_replace(());
}
```

<!-- FIXME: replace with `crates.io` link -->
[`vise`]: ../vise
[`hyper`]: https://crates.io/crates/hyper
