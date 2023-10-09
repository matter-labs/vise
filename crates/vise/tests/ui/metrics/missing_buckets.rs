use vise::{Histogram, Metrics};

#[derive(Debug, Metrics)]
struct TestMetrics {
    /// Test histogram.
    histogram: Histogram<u64>,
}

fn main() {}
