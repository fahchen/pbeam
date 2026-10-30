use std::{cell::RefCell, io};

use super::{SetErrors, prio_tos_trick};

/// Run `prio_tos_trick` with the saved values `saved`, where `results`
/// gives the results of the set, of the set of `tos` again, and of the set
/// of `priority` again. Return the errno of the result and the sets in
/// order.
fn trick(
    kind: libc::c_int,
    errors: SetErrors,
    saved: (io::Result<u32>, io::Result<u32>),
    results: [io::Result<()>; 3],
) -> (Option<i32>, Vec<String>) {
    let sets = RefCell::new(Vec::new());
    let [result, tos, priority] = results;
    let outcome = prio_tos_trick(
        kind,
        errors,
        saved,
        || {
            sets.borrow_mut().push("set".to_owned());
            result
        },
        |value| {
            sets.borrow_mut().push(format!("tos {value}"));
            tos
        },
        |value| {
            sets.borrow_mut().push(format!("priority {value}"));
            priority
        },
    );
    let errno = outcome.err().map(|failure| {
        failure
            .raw_os_error()
            .expect("an option error has an errno")
    });
    (errno, sets.into_inner())
}

fn errno<T>(code: i32) -> io::Result<T> {
    Err(io::Error::from_raw_os_error(code))
}

#[test]
fn sets_tos_and_priority_again_after_another_option() {
    let (errno, sets) = trick(
        libc::IP_TTL,
        SetErrors::Propagate,
        (Ok(3), Ok(16)),
        [Ok(()), Ok(()), Ok(())],
    );
    assert_eq!(errno, None);
    assert_eq!(sets, ["set", "tos 16", "priority 3"]);
}

#[test]
fn sets_only_priority_again_after_an_option_with_the_constant_of_ip_tos() {
    // `TCP_NODELAY` and `IP_TOS` are both 1 (inet_drv.c:6799).
    assert_eq!(libc::TCP_NODELAY, libc::IP_TOS);
    let (errno, sets) = trick(
        libc::TCP_NODELAY,
        SetErrors::Propagate,
        (Ok(3), Ok(16)),
        [Ok(()), Ok(()), Ok(())],
    );
    assert_eq!(errno, None);
    assert_eq!(sets, ["set", "priority 3"]);
}

#[test]
fn sets_nothing_again_after_priority_or_a_failed_set() {
    let ok = || [Ok(()), Ok(()), Ok(())];
    assert_eq!(
        trick(
            libc::SO_PRIORITY,
            SetErrors::Propagate,
            (Ok(3), Ok(16)),
            ok()
        ),
        (None, vec!["set".to_owned()])
    );
    let failed = trick(
        libc::IP_TTL,
        SetErrors::Propagate,
        (Ok(3), Ok(16)),
        [errno(libc::EINVAL), Ok(()), Ok(())],
    );
    assert_eq!(failed, (Some(libc::EINVAL), vec!["set".to_owned()]));
    let unsaved = trick(
        libc::IP_TTL,
        SetErrors::Propagate,
        (errno(libc::EBADF), errno(libc::EBADF)),
        ok(),
    );
    assert_eq!(unsaved, (None, vec!["set".to_owned()]));
}

#[test]
fn gives_a_failed_set_of_tos_again_only_with_propagate() {
    let results = || [Ok(()), errno(libc::EINVAL), Ok(())];
    let propagated = trick(
        libc::IP_TTL,
        SetErrors::Propagate,
        (Ok(3), Ok(16)),
        results(),
    );
    assert_eq!(
        propagated,
        (
            Some(libc::EINVAL),
            vec!["set".to_owned(), "tos 16".to_owned()]
        )
    );
    let (errno, sets) = trick(
        libc::SO_KEEPALIVE,
        SetErrors::Ignore,
        (Ok(3), Ok(16)),
        results(),
    );
    assert_eq!(errno, None);
    assert_eq!(sets, ["set", "tos 16", "priority 3"]);
}

#[test]
fn gives_a_failed_set_of_priority_again_with_propagate_unless_it_is_eperm() {
    // Some kernels do not permit a reset of `SO_PRIORITY` (inet_drv.c:6815-6818).
    let failed = |code| [Ok(()), Ok(()), errno(code)];
    let saved = || (Ok(3), Ok(16));
    assert_eq!(
        trick(
            libc::IP_TTL,
            SetErrors::Propagate,
            saved(),
            failed(libc::EPERM)
        )
        .0,
        None
    );
    assert_eq!(
        trick(
            libc::IP_TTL,
            SetErrors::Propagate,
            saved(),
            failed(libc::EINVAL)
        )
        .0,
        Some(libc::EINVAL)
    );
    assert_eq!(
        trick(
            libc::SO_KEEPALIVE,
            SetErrors::Ignore,
            saved(),
            failed(libc::EINVAL)
        )
        .0,
        None
    );
}
