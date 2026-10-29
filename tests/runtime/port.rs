use std::os::fd::AsRawFd;

use super::*;

#[test]
fn stops_the_read_select_of_an_fd_port_after_a_failed_read() {
    // A `read` of a directory fails with `EISDIR`; `ready_input` then
    // stops the read select, and the port stays open
    // (sys_drivers.c:1339-1342, 1211-1214).
    let directory = std::fs::File::open("/").unwrap();
    let mut spawn = Spawn {
        exit_status: false,
        soft_eof: true,
        packets: Packets::new(0),
        binary: true,
        lines: None,
        parallelism: false,
        program: Program::Fds(Fds {
            input: directory.as_raw_fd(),
            output: 1,
        }),
        input: Input::Open,
        queue: OutputQueue::new(4096, 8192),
    };
    let mut input = 0;
    assert!(read_spawn(&mut spawn, &mut input).is_none());
    assert_eq!(spawn.input, Input::Ended);
    assert_eq!(input, 0);
}
