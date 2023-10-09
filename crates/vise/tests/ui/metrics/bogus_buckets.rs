use vise::{Histogram, Metrics};

#[derive(Debug, Metrics)]
struct TestMetrics {
    /// Test histogram.
    #[metrics(buckets = "42")]
    histogram: Histogram<u64>,
}

fn main() {}
