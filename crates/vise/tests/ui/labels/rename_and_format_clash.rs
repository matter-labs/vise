use vise::EncodeLabelValue;

#[derive(Debug, EncodeLabelValue)]
#[metrics(rename_all = "snake_case", format = "{:?}")]
enum Label {
    Test,
    Value,
}

fn main() {}
