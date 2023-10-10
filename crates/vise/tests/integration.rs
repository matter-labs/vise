//! Integration tests for `vise` library.

#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/metrics/*.rs");
    t.compile_fail("tests/ui/labels/*.rs");
}
