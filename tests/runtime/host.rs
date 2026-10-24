//! Unit tests of `src/runtime/host.rs` for a host value that a fixture
//! cannot choose: the `uname` release text of the CI host starts each part
//! with a digit.

use super::*;

#[test]
fn reads_the_release_numbers_as_get_number_does() {
    // `get_number` (`sys/unix/sys.c:860-885`) gives 0 for a part that does
    // not start with a digit and does not move past it, so the later
    // numbers are 0 too.
    assert_eq!(release_numbers(b"6.8.0-1017-azure"), [6, 8, 0]);
    assert_eq!(release_numbers(b"24.1"), [24, 1, 0]);
    assert_eq!(release_numbers(b"x.2.3"), [0, 0, 0]);
}
