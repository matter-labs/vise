# Metrics Exporter for `vise`

[![Build Status](https://github.com/matter-labs/vise/workflows/Rust/badge.svg?branch=main)](https://github.com/matter-labs/vise/actions)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%2FApache--2.0-blue)](https://github.com/matter-labs/vise#license)
![rust 1.70+ required](https://img.shields.io/badge/rust-1.70+-blue.svg?label=Required%20Rust)

**Documentation:**
[![crate docs (main)](https://img.shields.io/badge/main-yellow.svg?label=docs)](https://matter-labs.github.io/vise/vise_exporter/)

This crate provides a simple [Prometheus] metrics exporter for metrics defined
using [`vise`]. It is based on the [`hyper`] library and supports both pull-based
and push-based communication with Prometheus.

## Usage

Add this to your Crate.toml:

```toml
[dependencies]
vise-exporter = "0.1.0"
```

An exporter can be initialized from a metrics `Registry`:

```rust
use tokio::sync::watch;

use vise_exporter::MetricsExporter;

async fn my_app() {
    let (shutdown_sender, mut shutdown_receiver) = watch::channel(());
    let exporter = MetricsExporter::default()
        .with_graceful_shutdown(async move {
            shutdown_receiver.changed().await.ok();
        });
    let bind_address = "0.0.0.0:3312".parse().unwrap();
    tokio::spawn(exporter.start(bind_address));

    // Then, once the app is shutting down:
    shutdown_sender.send_replace(());
}
```

See crate docs for more examples.

## License

Distributed under the terms of either

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

[prometheus]: https://prometheus.io/docs/introduction/overview/
<!-- FIXME: replace with `crates.io` link -->
[`vise`]: ../vise
[`hyper`]: https://crates.io/crates/hyper
