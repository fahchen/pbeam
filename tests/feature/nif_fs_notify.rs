//! `fs_notify` 0.1.2 from Hex runs through the PBeam NIF ABI.
//!
//! `fixtures/fs_notify` sets `config :rustler_precompiled, force_build_all:
//! true` and the rustler feature `nif_version_2_18`, so rustler builds the
//! NIF library of `fs_notify` from source with the host `cargo`, as Suikou
//! does. The test builds the release of the project, checks that the NIF
//! gate of `pbeam_bundle` accepts its bundle, and runs the one-file
//! executable with the entry `FsNotifyFixture.main/0`. The entry watches a
//! new temporary directory, writes a file, and waits for the event, which a
//! thread of the library sends (`enif_send` with no caller env). The line
//! has the base names of the paths, so the temporary directory does not
//! change it. The PBeam line is the line of the pinned ERTS.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

#[path = "../common/mix_bundle.rs"]
mod mix_bundle;
#[path = "../common/run.rs"]
#[expect(
    dead_code,
    reason = "this test crate keeps the stdout of the run and its locale"
)]
mod run;

use mix_bundle::{Release, run_release_bundle};

#[test]
fn runs_a_bundle_that_gets_a_file_event_from_a_thread_of_the_library() {
    let expected = run_release_bundle(&Release {
        project: "fixtures/fs_notify",
        app: "fs_notify_fixture",
        entry: "FsNotifyFixture",
        library: "lib/fs_notify-0.1.2/priv/native/fs_notify.so",
    });
    assert_eq!(expected, "{{:create, :file, [\"hello.txt\"]}, :ok}\n");
}
