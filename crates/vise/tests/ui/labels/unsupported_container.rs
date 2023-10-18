use vise::{EncodeLabelSet, EncodeLabelValue};

#[derive(Debug, EncodeLabelSet)]
enum Unsupported {}

#[derive(Debug, EncodeLabelSet)]
struct UnsupportedToo(u8);

#[derive(Debug, EncodeLabelValue)]
#[metrics(rename_all = "snake_case")]
struct UnsupportedForValue {
    test: u8,
}

fn main() {}
