use vise::Metrics;

#[derive(Debug, Metrics)]
enum Unsupported {}

#[derive(Debug, Metrics)]
struct UnsupportedToo(u8);

fn main() {}
