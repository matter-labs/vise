use vise::EncodeLabelValue;

#[derive(Debug, EncodeLabelValue)]
#[metrics(rename_all = "snake_case")]
enum Label {
    Test,
    #[metrics(rename = "_")]
    Other,
}

fn main() {}
