use wavyte::Composition;

#[test]
fn json_fixture_validates() {
    let s = include_str!("data/simple_comp.json");
    let comp: Composition = serde_json::from_str(s).unwrap();
    comp.validate().unwrap();
}
