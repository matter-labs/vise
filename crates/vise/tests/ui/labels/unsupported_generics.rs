use vise::EncodeLabelValue;

#[derive(Debug, EncodeLabelValue)]
struct UnsupportedLifetime<'a> {
    label: &'a str,
}

#[derive(Debug, EncodeLabelValue)]
struct UnsupportedConstParam<const N: usize> {
    labels: [&'static str; N],
}

#[derive(Debug, EncodeLabelValue)]
struct UnsupportedTypeParam<T> {
    counter: T,
}

fn main() {}
