use vise::{Counter, LabeledFamily, Metrics};

#[derive(Debug, Metrics)]
struct TestMetrics {
    #[metrics(labels = ["methoD"])]
    histograms: LabeledFamily<&'static str, Counter>,
}

fn main() {}
