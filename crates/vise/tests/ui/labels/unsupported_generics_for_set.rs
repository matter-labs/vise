use vise::EncodeLabelSet;

#[derive(Debug, EncodeLabelSet)]
struct UnsupportedLifetime<'a> {
    label: &'a str,
}

#[derive(Debug, EncodeLabelSet)]
struct UnsupportedConstParam<const N: usize> {
    labels: [&'static str; N],
}

#[derive(Debug, EncodeLabelSet)]
struct UnsupportedTypeParam<T> {
    counter: T,
}

fn main() {}
