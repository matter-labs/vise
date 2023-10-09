use vise::Metrics;

#[derive(Debug, Metrics)]
struct TestMetrics {
    /// Test counter.
    counter: u8,
}

fn main() {}
