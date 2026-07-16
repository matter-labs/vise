//! Integration tests for `vise` library.

/// UI tests assert on the exact text of `rustc` diagnostics, which is **not** stable across
/// compiler releases. Running them on an arbitrary toolchain produces spurious mismatches, so
/// they are opt-in: they only run when `VISE_UI_TESTS` is set in the environment. CI runs them
/// under a single pinned toolchain (see the `ui-tests` job in `.github/workflows/rust.yml`).
///
/// To refresh the expected `*.stderr` snapshots after an intentional change, run with the same
/// pinned toolchain that CI uses:
///
/// ```bash
/// VISE_UI_TESTS=1 TRYBUILD=overwrite cargo +<pinned-version> test -p vise --test integration
/// ```
#[test]
fn ui() {
    if std::env::var_os("VISE_UI_TESTS").is_none() {
        eprintln!(
            "skipping UI tests; set `VISE_UI_TESTS=1` to run them (only reliable on the pinned \
             toolchain, see `.github/workflows/rust.yml`)"
        );
        return;
    }

    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/metrics/*.rs");
    t.compile_fail("tests/ui/labels/*.rs");
}
