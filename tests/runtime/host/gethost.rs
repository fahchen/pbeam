use super::*;

#[test]
fn maps_the_netdb_errors_as_map_netdb_error() {
    // inet_gethost.c:2008-2037; the resolver of the host gives these
    // values only for a failing name server.
    let codes = [HOST_NOT_FOUND, TRY_AGAIN, NO_RECOVERY, NO_DATA, 99].map(netdb_code);
    assert_eq!(
        codes,
        [
            Code::HostNotFound,
            Code::TryAgain,
            Code::NoRecovery,
            Code::NoData,
            Code::Internal
        ]
    );
}

#[test]
fn maps_the_address_info_errors_as_map_netdb_error_ai() {
    // inet_gethost.c:2039-2070.
    let codes = [
        libc::EAI_AGAIN,
        libc::EAI_FAIL,
        libc::EAI_NONAME,
        EAI_NODATA,
        libc::EAI_MEMORY,
    ]
    .map(address_info_code);
    assert_eq!(
        codes,
        [
            Code::TryAgain,
            Code::HostNotFound,
            Code::HostNotFound,
            Code::HostNotFound,
            Code::Internal
        ]
    );
}

#[test]
fn gives_the_error_strings_of_errcode_to_string() {
    // inet_gethost.c:2074-2097.
    let replies = [
        Code::HostNotFound,
        Code::TryAgain,
        Code::NoRecovery,
        Code::NoData,
        Code::Internal,
    ]
    .map(error_reply);
    assert_eq!(
        replies,
        [
            b"\0notfound\0".to_vec(),
            b"\0try_again\0".to_vec(),
            b"\0no_recovery\0".to_vec(),
            b"\0no_data\0".to_vec(),
            b"\0netdb_internal\0".to_vec()
        ]
    );
}

#[test]
fn stops_at_a_failed_read_or_write() {
    // A read error is a fatal error of the main loop
    // (inet_gethost.c:1330-1347); a failed write ends the worker
    // (inet_gethost.c:1989-1990).
    struct Failing;
    impl Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("read"))
        }
    }
    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("write"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert_eq!(serve(&mut Failing, &mut Vec::new()), 1);
    let request = b"\0\0\0\x0a\0\0\0\x01\x02\x01\x7f\0\0\x01";
    assert_eq!(serve(&mut request.as_slice(), &mut Failing), 1);
}

#[test]
fn reads_again_after_an_interrupted_read() {
    // read_exact retries EINTR (inet_gethost.c:2504-2506).
    struct Interrupted(bool);
    impl Read for Interrupted {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            if std::mem::replace(&mut self.0, true) {
                Ok(0)
            } else {
                Err(std::io::ErrorKind::Interrupted.into())
            }
        }
    }
    assert_eq!(serve(&mut Interrupted(false), &mut Vec::new()), 0);
}
