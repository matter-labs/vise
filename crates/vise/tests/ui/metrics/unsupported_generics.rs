use vise::Metrics;

#[derive(Debug, Metrics)]
struct UnsupportedLifetime<'a> {
    label: &'a str,
}

#[derive(Debug, Metrics)]
struct UnsupportedConstParam<const N: usize> {
    labels: [&'static str; N],
}

#[derive(Debug, Metrics)]
struct UnsupportedTypeParam<T> {
    counter: T,
}

fn main() {}
