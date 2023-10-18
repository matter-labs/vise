use vise::EncodeLabelValue;

#[derive(Debug, EncodeLabelValue)]
#[metrics(rename_all = "snake_case")]
enum Label {
    Test,
    #[metrics(name = "test")]
    Other,
}

fn main() {}
