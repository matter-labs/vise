use vise::EncodeLabelValue;

#[derive(Debug, EncodeLabelValue)]
#[metrics(rename_all = "what")]
enum Label {
    Test,
    Value,
}

fn main() {}
