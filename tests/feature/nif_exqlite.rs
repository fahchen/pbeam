//! exqlite 0.37.0 from Hex runs through the PBeam NIF ABI.
//!
//! `fixtures/exqlite` sets `config :exqlite, force_build: true`, so
//! `elixir_make` builds `sqlite3_nif.so` from source with the pinned ERTS
//! headers and the host `cc`, as Suikou does. The test builds the release of
//! the project, checks that the NIF gate of `pbeam_bundle` accepts its
//! bundle, and runs the one-file executable with the entry
//! `ExqliteFixture.main/0`. The entry opens an in-memory database, creates
//! a table, inserts rows with bound parameters of each type, and reads them
//! back. The PBeam line is the line of the pinned ERTS.
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
fn runs_a_bundle_that_reads_an_in_memory_sqlite_database() {
    let expected = run_release_bundle(&Release {
        project: "fixtures/exqlite",
        app: "exqlite_fixture",
        entry: "ExqliteFixture",
        library: "lib/exqlite-0.37.0/priv/sqlite3_nif.so",
    });
    assert_eq!(
        expected,
        "{:ok, [:done, :done], {:ok, 1}, {:ok, [\"id\", \"name\", \"n\", \"x\", \"b\", \"z\"]}, \
         {:ok, [[1, \"one\", 42, 1.5, <<0, 1, 255>>, nil], [2, \"two\", -7, -0.25, \"\", \"z\"]]}, :ok}\n"
    );
}
