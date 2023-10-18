use vise::{Counter, Metrics};

#[derive(Debug, Metrics)]
#[metrics(prefix = "what?")]
struct TestMetrics {
    /// Test counter.
    counter: Counter,
}

fn main() {}
