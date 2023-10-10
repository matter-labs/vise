use vise::EncodeLabelSet;

#[derive(Debug, EncodeLabelSet)]
struct Labels {
    test: &'static str,
    код: u16,
}

fn main() {}
