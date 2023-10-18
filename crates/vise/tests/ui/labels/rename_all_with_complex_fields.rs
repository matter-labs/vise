use vise::EncodeLabelValue;

#[derive(Debug, EncodeLabelValue)]
#[metrics(rename_all = "snake_case")]
enum Label {
    Test,
    Value(u8),
}

fn main() {}
