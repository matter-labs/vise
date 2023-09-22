# E2E Test for `vise-exporter`

This crate implements a mock app that defines `vise` metrics and uses the corresponding exporter.
This app is run in [integration tests](tests/integration.rs) which then spawn a Prometheus instance
in a Docker container (via [Testcontainers]), wait until the app is scraped, and check using
Prometheus [HTTP API][prom-api] (via [`prometheus-http-query`]) that metrics have expected values
and metadata.

To run these tests, you need to have Docker installed locally.

[Testcontainers]: https://testcontainers.com/
[prom-api]: https://prometheus.io/docs/prometheus/latest/querying/api/
[`prometheus-http-query`]: https://crates.io/crates/prometheus-http-query
