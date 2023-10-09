use vise::{Counter, Metrics};

#[derive(Debug, Metrics)]
struct TestMetrics {
    /// Test counter.
    #[metrics(what = 42)]
    counter: Counter,
}

fn main() {}
