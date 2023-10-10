use vise::{EncodeLabelSet, EncodeLabelValue};

#[derive(Debug, EncodeLabelSet, EncodeLabelValue)]
#[metrics(label = "what?", rename_all = "snake_case")]
enum Label {
    Test,
    Value,
}

fn main() {}
