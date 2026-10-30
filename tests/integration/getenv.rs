//! Runs `os:getenv` on PBeam and compares the results with the pinned ERTS.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::{
    env,
    ffi::OsStr,
    fs,
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    process::{Command, Stdio},
};

use pbeam_runtime::BeamProgram;

const GETENV_PROBE: &str = "fixtures/getenv/getenv_probe.beam";

/// The environment variable that gives the `isolated_getenv` child the
/// file for its result.
const RESULT: &str = "PBEAM_GETENV_RESULT";

/// The environment of `getenv_probe` as raw bytes. The test sets it on each
/// child `Command`, and never in the test process.
const VARIABLES: [(&[u8], &[u8]); 8] = [
    (b"PBEAM_GETENV_ASCII", b"yes"),
    (b"PBEAM_GETENV_EMPTY", b""),
    (b"PBEAM_GETENV_COMPOSED", b"a\xc3\xa9"),
    (b"PBEAM_GETENV_INVALID", b"a\xff"),
    (b"PBEAM_GETENV_CYRILLIC", b"\xd0\x96"),
    (b"PBEAM_GETENV_EQUALS", b"x=y"),
    (b"PBEAM_GETENV_DECOMPOSED", b"ae\xcc\x81"),
    (b"PBEAM_\xc3\xa9", b"k"),
];

/// One result of `getenv_probe`.
#[derive(Clone, Copy)]
enum Value {
    /// A list of characters.
    Characters(&'static [i64]),
    /// The atom `false`.
    False,
    /// The exception `error:badarg`, which `keys/0` catches.
    Badarg,
}

impl Value {
    /// Return the `~w` text of the value on ERTS.
    fn erts(self) -> String {
        match self {
            Value::Characters(characters) => format!(
                "[{}]",
                characters
                    .iter()
                    .map(i64::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Value::False => "false".to_owned(),
            Value::Badarg => "{error,badarg}".to_owned(),
        }
    }

    /// Return the `Debug` text of the value in PBeam.
    fn pbeam(self) -> String {
        match self {
            Value::Characters(characters) => format!(
                "List([{}])",
                characters
                    .iter()
                    .map(|character| format!("Integer({character})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Value::False => "Atom(0)".to_owned(),
            Value::Badarg => "Tuple([Atom(12), Atom(78)])".to_owned(),
        }
    }
}

const YES: Value = Value::Characters(&[121, 101, 115]);

/// The native file name encoding of the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeEncoding {
    /// UTF-8.
    Utf8,
    /// Latin-1.
    Latin1,
}

/// Return the results of `keys/0`, `values/0`, and `decomposed/0` on ERTS
/// in the native file name encoding of the host. The encoding is UTF-8 on
/// macOS and with `LC_ALL=C.UTF-8`, else Latin-1
/// (erl_sys_common_misc.c:89-126). On macOS, ERTS composes the code points
/// of a value (erl_unicode.c:2177-2190), so `decomposed/0` gives [97,233].
/// In Latin-1, the key `PBEAM_é` is the byte `\xe9`, which is not in the
/// environment, and `[256]` raises `badarg`. The values come from
/// erl_unicode.c:2164-2220.
fn expected(encoding: NativeEncoding) -> (Vec<Value>, Vec<Value>, Value) {
    use Value::Characters;
    let (key_results, composed, cyrillic, decomposed) = if encoding == NativeEncoding::Utf8 {
        let decomposed = if cfg!(target_os = "macos") {
            Characters(&[97, 233])
        } else {
            Characters(&[97, 101, 769])
        };
        (
            [Characters(&[107]), Value::False],
            Characters(&[97, 233]),
            Characters(&[1046]),
            decomposed,
        )
    } else {
        (
            [Value::False, Value::Badarg],
            Characters(&[97, 195, 169]),
            Characters(&[208, 150]),
            Characters(&[97, 101, 204, 129]),
        )
    };
    let mut keys = vec![YES, YES, YES, Value::False, Value::False];
    keys.extend(key_results);
    keys.extend([Value::Badarg; 14]);
    let values = vec![
        Characters(&[]),
        composed,
        Characters(&[97, 255]),
        cyrillic,
        Characters(&[120, 61, 121]),
    ];
    (keys, values, decomposed)
}

/// Set the `getenv_probe` environment and `LC_ALL` on `command`.
fn environment<'a>(command: &'a mut Command, locale: &str) -> &'a mut Command {
    for (name, value) in VARIABLES {
        command.env(OsStr::from_bytes(name), OsStr::from_bytes(value));
    }
    command.env("LC_ALL", locale).stdin(Stdio::null())
}

/// Return the path of the first `erl` script in `PATH`.
fn erl() -> PathBuf {
    env::split_paths(&env::var_os("PATH").unwrap())
        .map(|directory| directory.join("erl"))
        .find(|path| path.is_file())
        .unwrap()
}

#[test]
fn reads_the_os_environment_with_os_getenv_as_erts() {
    for locale in ["C.UTF-8", "C"] {
        let utf8 = cfg!(target_os = "macos") || locale == "C.UTF-8";
        let encoding = if utf8 {
            NativeEncoding::Utf8
        } else {
            NativeEncoding::Latin1
        };
        let (keys, values, decomposed) = expected(encoding);
        let erts_list = |values: &[Value]| {
            format!(
                "[{}]",
                values
                    .iter()
                    .map(|value| value.erts())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        // Run the `erl` script with bash. The script starts with
        // `#!/bin/sh`, and dash, the `/bin/sh` of Ubuntu, removes the
        // variable `PBEAM_é` from the environment because its name is not
        // a shell name. Bash keeps the variable.
        let erts = environment(&mut Command::new("bash"), locale)
            .arg(erl())
            .args([
                "-noshell",
                "-pa",
                "fixtures/getenv",
                "-eval",
                "io:format(\"~w~n~w~n~w~n\", [getenv_probe:keys(), getenv_probe:values(), getenv_probe:decomposed()]), halt().",
            ])
            .output()
            .unwrap();
        assert!(erts.status.success(), "{locale}: {erts:?}");
        assert_eq!(
            String::from_utf8(erts.stdout).unwrap(),
            format!(
                "{}\n{}\n{}\n",
                erts_list(&keys),
                erts_list(&values),
                decomposed.erts()
            ),
            "{locale}"
        );

        let directory = env::temp_dir().join(format!("pbeam-getenv-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let result = directory.join("result");
        let child = environment(&mut Command::new(env::current_exe().unwrap()), locale)
            .args(["--exact", "isolated_getenv", "--include-ignored", "--quiet"])
            .env(RESULT, &result)
            .output()
            .unwrap();
        let written = fs::read_to_string(&result);
        fs::remove_dir_all(&directory).unwrap();
        assert!(
            child.status.success(),
            "{locale}: child exited with {}",
            child.status
        );
        let pbeam_list = |values: &[Value]| {
            format!(
                "Ok(List([{}]))",
                values
                    .iter()
                    .map(|value| value.pbeam())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        // PBeam has no composition table, so on macOS `decomposed/0`
        // returns a runtime error (see "OS environment" in
        // docs/erts-differences.md).
        let decomposed = if cfg!(target_os = "macos") {
            "Err(Runtime(\"unsupported composition of the native name code point 769 on macOS: see \\\"OS environment\\\" in docs/erts-differences.md\"))".to_owned()
        } else {
            format!("Ok({})", decomposed.pbeam())
        };
        assert_eq!(
            written.unwrap(),
            format!(
                "{}\n{}\n{decomposed}",
                pbeam_list(&keys),
                pbeam_list(&values)
            ),
            "{locale}"
        );
    }
}

/// The child side of `reads_the_os_environment_with_os_getenv_as_erts`:
/// run `keys/0`, `values/0`, and `decomposed/0` of `getenv_probe` and write
/// the `Debug` text of each result to the result file. A child process is
/// necessary, because the test sets the environment and the locale only on
/// the child `Command`, and PBeam reads the file name encoding once for
/// each process. The child inherits `LLVM_PROFILE_FILE`, so
/// `cargo llvm-cov` measures it.
#[test]
#[ignore = "reads_the_os_environment_with_os_getenv_as_erts runs it in a child process"]
fn isolated_getenv() {
    let probe = fs::read(GETENV_PROBE).unwrap();
    let program = BeamProgram::load(&[&probe]).unwrap();
    let results: Vec<String> = ["keys", "values", "decomposed"]
        .into_iter()
        .map(|function| format!("{:?}", program.execute("getenv_probe", function, &[])))
        .collect();
    fs::write(env::var_os(RESULT).unwrap(), results.join("\n")).unwrap();
}
