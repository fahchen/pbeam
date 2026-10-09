use super::*;

#[test]
fn names_errno_values_as_erts() {
    // `errno_0` is the name of 0 (erl_errno_str.c:1161-1163). ENOENT and
    // EAGAIN are cases of `errno_name`, and EWOULDBLOCK has the value of
    // EAGAIN, so it gives the first name. EPERM is not a case of
    // `errno_name`, so the host lookup gives it. 10000 and -1 have no
    // name, so they give `errno_N` (erl_errno_str.c:47).
    for (errno, name) in [
        (0, "errno_0"),
        (libc::ENOENT, "enoent"),
        (libc::EAGAIN, "eagain"),
        (libc::EWOULDBLOCK, "eagain"),
        (libc::EPERM, "eperm"),
        (10000, "errno_10000"),
        (-1, "errno_-1"),
    ] {
        assert_eq!(errno_id(errno), name, "errno {errno}");
    }
}

#[test]
fn drops_a_guarded_case_whose_guard_name_has_its_value() {
    // On a host where `ENOTTY` has the value of `ENOSTR`, ERTS does not
    // compile the earlier `ENOSTR` case, so the name is `ENOTTY`
    // (erl_errno_str.c:1042-1046). Without a collision, `ENOSTR` stays.
    let collision: [(c_int, &str, &[c_int]); 2] = [(5, "ENOSTR", &[5]), (5, "ENOTTY", &[])];
    assert_eq!(case_name(&collision, 5), Some("ENOTTY"));
    let distinct: [(c_int, &str, &[c_int]); 2] = [(5, "ENOSTR", &[6]), (6, "ENOTTY", &[])];
    assert_eq!(case_name(&distinct, 5), Some("ENOSTR"));
    assert_eq!(case_name(&distinct, 7), None);
}
