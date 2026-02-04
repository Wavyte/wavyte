#[test]
fn svg_text_fixture_files_exist() {
    assert!(std::path::Path::new("tests/data/svg_with_text.svg").is_file());
    assert!(std::path::Path::new("tests/data/fonts/Inconsolata-Regular.ttf").is_file());
    assert!(std::path::Path::new("tests/data/fonts/OFL.txt").is_file());
}
