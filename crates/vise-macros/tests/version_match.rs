use version_sync::assert_html_root_url_updated;

#[test]
fn html_root_url_is_in_sync() {
    assert_html_root_url_updated!("src/lib.rs");
}
