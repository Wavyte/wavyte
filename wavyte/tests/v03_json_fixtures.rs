use std::fs;

use wavyte::Composition;

#[test]
fn load_and_validate_v03_fixtures() {
    for entry in fs::read_dir("tests/data/v03").unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let comp = Composition::from_path(&path).unwrap();
        comp.validate().unwrap();
    }
}
