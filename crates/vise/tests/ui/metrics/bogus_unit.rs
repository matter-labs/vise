use vise::{Counter, Metrics};

#[derive(Debug, Metrics)]
struct TestMetrics {
    /// Test counter.
    #[metrics(unit = "seconds")]
    counter: Counter,
}

fn main() {}
