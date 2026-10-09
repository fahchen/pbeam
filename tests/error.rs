use super::error;

#[test]
fn formats_runtime_errors() {
    assert_eq!(error("bad module").to_string(), "bad module");
}
