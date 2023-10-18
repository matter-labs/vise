use vise::{Counter, Metrics};

#[derive(Debug, Metrics)]
struct TestMetrics {
    /// Test counter.
    #[metrics(buckets = &[1.0, 2.0])]
    counter: Counter,
}

fn main() {}
