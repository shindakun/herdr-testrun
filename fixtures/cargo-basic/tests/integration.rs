#[test]
fn integration_panics() {
    let v: Vec<i32> = Vec::new();
    let _ = v[cargo_basic::add(0, 0) as usize];
}

#[test]
fn integration_passes() {
    assert_eq!(cargo_basic::add(2, 2), 4);
}
