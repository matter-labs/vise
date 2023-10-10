use vise::EncodeLabelSet;

#[derive(Debug, EncodeLabelSet)]
#[metrics(what = 42)]
struct LabelSet {
    method: &'static str,
}

fn main() {}
