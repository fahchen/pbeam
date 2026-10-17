use super::*;

#[test]
fn emits_the_expansion_and_rejects_a_failed_tparm() {
    // `tty_tputs_nif` gives `{error, {tparm, Errno}}` for a null result
    // (prim_tty_nif.c:956-962). A failed `tparm` needs an allocation
    // failure, so the test drives the null check directly. The string
    // "cup" has no padding, so `tputs` emits it without a terminal.
    let _guard = TPUTC_LOCK.lock().expect("the tputs lock is not poisoned");
    assert!(emit_expanded(std::ptr::null_mut()).is_err());
    assert_eq!(
        emit_expanded(c"cup".as_ptr().cast_mut()),
        Ok(b"cup".to_vec())
    );
}
