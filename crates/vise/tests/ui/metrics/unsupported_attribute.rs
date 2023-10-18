use vise::{Counter, Metrics};

#[derive(Debug, Metrics)]
#[metrics(what = 42)]
struct TestMetrics {
    /// Test counter.
    counter: Counter,
}

fn main() {}
