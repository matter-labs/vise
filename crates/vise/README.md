# Vise – Typesafe Metrics Client

This library provides a high-level wrapper for defining and reporting metrics in Rust libraries and applications.

## Features

- Allows registering and reporting metrics in an idiomatic and typesafe manner.
- Allows testing metrics by accessing their current values. (**Note:** Accessing metric data is not implemented for
  histograms yet.)

## What are metrics, anyway

Metrics are numerical measurements taken over time. Metrics are defined and collected in an application and reported to
an external system, [Prometheus], from which they can be accessed using e.g. [Grafana] dashboards.

Prometheus and compatible systems supports 3 main [metric types](https://prometheus.io/docs/concepts/metric_types/):

- **Counters** are monotonically increasing integer values
- **Gauges** are integer or floating-point values that can go up or down. Logically, a reported gauge value can be
  treated as valid until the next value is reported.
- **Histograms** are floating-point values counted in configurable buckets. Logically, a histogram observes a certain
  probability distribution, and observations are transient (unlike gauge values).

Metrics of all types can be supplied with _labels_. Each set of labels defines a separate metric. Thus, label space
should be reasonably small.

## Usage

### Defining and reporting metrics

Metrics are defined as structs, with each field corresponding to a metric or a family of metrics:

```rust
use vise::*;
use std::{fmt, time::Duration};

/// Metrics defined by the library or application. A single app / lib can define
/// multiple metric structs.
#[derive(Debug, Metrics)]
#[metrics(prefix = "my_app")]
// ^ Prefix added to all field names to get the final metric name (e.g., `my_app_latencies`).
pub(crate) struct MyMetrics {
    /// Simple counter. Doc comments for the fields will be reported
    /// as Prometheus metric descriptions.
    pub counter: Counter,
    /// Integer-valued gauge. Unit will be reported to Prometheus and will influence metric name
    /// by adding the corresponding suffix to it (in this case, `_bytes`).
    #[metrics(unit = Unit::Bytes)]
    pub gauge: Gauge<u64>,
    /// Group of histograms with the "method" label (see the definition below).
    /// Each `Histogram` or `Family` of `Histogram`s must define buckets; in this case,
    /// we use default buckets for latencies.
    #[metrics(buckets = Buckets::LATENCIES)]
    pub latencies: Family<Method, Histogram<Duration>>,
}

// Commonly, it makes sense to make metrics available using a static:
#[vise::register]
static MY_METRICS: Global<MyMetrics> = Global::new();

/// Isolated metric label. Note the `label` name specification below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelSet, EncodeLabelValue)]
#[metrics(label = "method")]
pub(crate) struct Method(pub &'static str);

// For the isolated metric label to work, you should implement `Display` for it:
impl fmt::Display for Method {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

// Metrics are singletons globally available using the `instance()` method.
MY_METRICS.counter.inc();
assert_eq!(MY_METRICS.counter.get(), 1); // Useful for testing

let latency = MY_METRICS.latencies[&Method("test")].start();
// Do some work...
let latency: Duration = latency.observe();
// `latency` can be used in logging etc.
```

See crate docs for more examples.

### Testing metrics

Depending on how you report metrics (e.g., whether the global state is used), testing metrics may require refactoring.

- You may pass around references to the metric type(s) in the logic under test so that these types can be injected and
  then checked by tests.
- Alternatively, your logic may produce _statistics_ that are then reported as metrics (this may be beneficial for
  performance as well). In this case, produced statistics can be checked by tests.

### Best practices

_See also: [Prometheus guidelines](https://prometheus.io/docs/practices/naming/)_

- Metrics and metric labels should be named in snake_case. (This should be enforced by Clippy and checks performed in
  the `Metrics` derive macro.)
- Metrics should start with a prefix or a sequence of prefixes describing the domain / subdomains owning the metric.
  Prefixes should be separated by a single `_` char.
- Metrics with a unit should have a corresponding suffix (e.g., `_seconds`). This suffix is automatically added to the
  metric name if you specify its unit; you **must not** specify it manually.
- Label names should not repeat the metric name and should not include units.
- Label values for each label should have reasonably low cardinality.
- If a label value encodes to a string (as opposed to an integer, integer range etc.), it should use snake_case.
- Metrics in a `Family` should have uniform meaning. If a `Family` can be documented without going into label specifics,
  you're usually on a right track.

#### Example: RocksDB size metrics

Suppose we want to report live and total data sizes for [RocksDB] instances that live in our application. We may want to
define:

- Families of gauges (since data sizes logically persist until the next size is reported)
- ...with `rocksdb_` prefix
- ...separate families for live and total data sizes (since they measure 2 distinct things)
- ...with `db` and `cf` labels specifying the database ID and column family name (the database ID should be globally
  unique; column families will probably differ among `db` values)
- ...with `Unit::Bytes` (since data sizes are measured in bytes)

Thus, we might have the following metrics:

```text
rocksdb_live_data_size_bytes{db="merkle_tree",cf="default"} 123456789
rocksdb_live_data_size_bytes{db="merkle_tree",cf="stale_keys"} 123456
rocksdb_total_data_size_bytes{db="merkle_tree",cf="default"} 130000000
rocksdb_total_data_size_bytes{db="merkle_tree",cf="stale_keys"} 130000
```

## License

Distributed under the terms of either

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

[prometheus]: https://prometheus.io/docs/introduction/overview/
[grafana]: https://grafana.com/docs/grafana/latest/
[rocksdb]: https://rocksdb.org/
