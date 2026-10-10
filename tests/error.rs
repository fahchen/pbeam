use super::{BeamError, error};

#[test]
fn formats_runtime_errors_and_halts() {
    assert_eq!(error("bad module").to_string(), "bad module");
    assert_eq!(BeamError::Halt(3).to_string(), "halted with status 3");
}
