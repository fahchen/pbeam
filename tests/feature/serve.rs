//! Serve `GET /` from `examples/plug_hello` on the pinned ERTS and on the
//! `pbeam` binary, and compare the status line, the headers without `date`,
//! and the body. Each run boots the release with its boot script, as
//! `bin/plug_hello start` does: the pinned ERTS with the `erl` of the
//! release, and PBeam with `pbeam erl`. The one-file run also compares the
//! SIGTERM shutdown notice and the exit status with the pinned ERTS. The
//! example starts Bandit on the port of the `PORT` variable, so each run
//! gets a free port and the runs do not collide. The ERTS run and the pbeam
//! run also serve `GET /` with `accept-encoding: zstd` first, which makes
//! Bandit compress the body with `zstd:compress/2` and a context, and
//! compare the compressed response.

use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// The status line, the headers without `date` in their order, and the
/// body of the response of the example to `GET /`, as the pinned ERTS
/// gives them.
const RESPONSE: (&str, &str, &str) = (
    "HTTP/1.1 200 OK",
    "connection: close\n\
     content-length: 23\n\
     vary: accept-encoding\n\
     cache-control: max-age=0, private, must-revalidate\n\
     content-type: text/plain; charset=utf-8\n",
    "Hello world from PBeam!",
);

/// The request of `GET /` with no `accept-encoding` header.
const PLAIN_REQUEST: &[u8] = b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";

/// The request of `GET /` that accepts only the `zstd` content encoding.
const ZSTD_REQUEST: &[u8] =
    b"GET / HTTP/1.1\r\nHost: localhost\r\nAccept-Encoding: zstd\r\nConnection: close\r\n\r\n";

/// A response: the status line, the headers without `date`, and the body.
type Response<Body> = (String, String, Body);

/// The variables that `pbeam erl` rejects when they are set and not empty,
/// as the flags of `erlexec` that PBeam does not read. Each run removes
/// them, so that a shell setting does not change the runs.
const ERL_FLAG_VARIABLES: [&str; 5] = [
    "ERL_OTP29_FLAGS",
    "ERL_AFLAGS",
    "ERL_FLAGS",
    "ERL_ZFLAGS",
    "ERL_EPMD_PORT",
];

/// The modules of the NIF libraries of PBeam, and `pubkey_os_cacerts`,
/// whose `on_load` calls `erlang:load_nif/2` only on Windows
/// (`pubkey_os_cacerts.erl:265-275`).
const NIF_MODULES: &str = "[asn1rt_nif, crypto, erl_tracer, prim_buffer, prim_file, \
                           prim_net, prim_socket, prim_tty, pubkey_os_cacerts, zlib, zstd]";

/// The time that the server has to start, and the time that a response has
/// to arrive.
const START_TIME_LIMIT: Duration = Duration::from_secs(120);
const READ_TIME_LIMIT: Duration = Duration::from_secs(60);

#[test]
fn serves_hello_world_from_the_plug_example_as_pinned_erts() {
    // The ERTS run builds the release and runs its entry with `erl +S 1`.
    // The pbeam run uses the same build (`--no-build`), so both runs
    // execute the same compiled modules. The one-file run appends the
    // build to a `pbeam` binary and starts it in a directory that holds no
    // other file.
    let (erts, erts_zstd) = serve(&["plug_hello", "--erts"], None);
    let (pbeam, pbeam_zstd) = serve(
        &["plug_hello", "--no-build"],
        Some(env!("CARGO_BIN_EXE_pbeam")),
    );
    let bundled = serve_one_file();
    // The NIF record of this acceptance: each module of a NIF library that
    // the release boot loads. A module with an `on_load` function is loaded
    // only when the function returns `ok`, so `erlang:load_nif/2` gave `ok`
    // for each library that the record names.
    let erts_record = nif_record(None);
    let pbeam_record = nif_record(Some(env!("CARGO_BIN_EXE_pbeam")));
    eprintln!("NIF record: {pbeam_record}");
    assert_eq!(erts_record, pbeam_record);
    assert_eq!(
        pbeam_record,
        "[{asn1rt_nif,true},{crypto,true},{erl_tracer,true},{prim_buffer,true},\
         {prim_file,true},{prim_net,true},{prim_socket,true},{prim_tty,true},\
         {pubkey_os_cacerts,true},{zlib,true},{zstd,true}]"
    );
    // The zstd response holds the bytes of libzstd 1.5.7, the version of
    // pinned ERTS, and the run serves the plain request after it.
    assert_eq!(erts_zstd, pbeam_zstd);
    assert!(
        pbeam_zstd.1.contains("\ncontent-encoding: zstd\n"),
        "{}",
        pbeam_zstd.1
    );
    assert_eq!(erts, pbeam);
    assert_eq!(pbeam, bundled);
    assert_eq!(
        pbeam,
        (
            RESPONSE.0.to_owned(),
            RESPONSE.1.to_owned(),
            RESPONSE.2.to_owned(),
        )
    );
}

#[test]
fn reports_an_embedded_bundle_that_does_not_parse() {
    // A binary with a trailer that names bytes that are not a bundle stops
    // with a message, before it starts a release.
    let directory = test_directory("parse-error");
    let mut bytes = fs::read(env!("CARGO_BIN_EXE_pbeam")).unwrap();
    bytes.extend_from_slice(b"garbage!");
    bytes.extend_from_slice(b"PBEAMEMB");
    bytes.extend_from_slice(&8_u64.to_le_bytes());
    let path = directory.join("broken");
    fs::write(&path, &bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new(&path)
        .stdin(Stdio::null())
        .output()
        .expect("the broken binary runs");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("does not parse"),
        "{:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn reports_a_run_that_the_entry_cannot_start() {
    // The entry module is not in the bundle, so the boot of the extracted
    // release fails, and the embedded mode returns its status.
    let directory = test_directory("entry-error");
    let mut writer =
        pbeam_runtime::BundleWriter::new(env!("PBEAM_OTP_RELEASE"), "test", "m", "f", 0).unwrap();
    writer.add_resource("meta/start-modules", b"init").unwrap();
    writer
        .add_resource("lib/kernel-1/ebin/init.beam", b"not a beam")
        .unwrap();
    writer
        .add_resource("releases/start_erl.data", b"17.1 0.1.0")
        .unwrap();
    let bundle = writer.encode();
    let mut bytes = fs::read(env!("CARGO_BIN_EXE_pbeam")).unwrap();
    bytes.extend_from_slice(&bundle);
    bytes.extend_from_slice(b"PBEAMEMB");
    bytes.extend_from_slice(&(bundle.len() as u64).to_le_bytes());
    let path = directory.join("incomplete");
    fs::write(&path, &bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new(&path)
        .stdin(Stdio::null())
        .output()
        .expect("the incomplete binary runs");
    assert_eq!(output.status.code(), Some(1));
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn reports_a_bundle_that_cannot_start() {
    // A bundle without a start set stops with the message of the start, and
    // the exit status of a failed run.
    let directory = test_directory("start-error");
    let writer =
        pbeam_runtime::BundleWriter::new(env!("PBEAM_OTP_RELEASE"), "test", "m", "f", 0).unwrap();
    let bundle = writer.encode();
    let mut bytes = fs::read(env!("CARGO_BIN_EXE_pbeam")).unwrap();
    bytes.extend_from_slice(&bundle);
    bytes.extend_from_slice(b"PBEAMEMB");
    bytes.extend_from_slice(&(bundle.len() as u64).to_le_bytes());
    let path = directory.join("incomplete");
    fs::write(&path, &bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new(&path)
        .stdin(Stdio::null())
        .output()
        .expect("the incomplete binary runs");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("meta/start-modules"),
        "{:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn builds_the_one_file_executable_with_the_mix_task_and_runs_it_clean() {
    // `mix pbeam.build` builds the runtime and the bundle tool with Cargo and
    // writes one executable. The test gives the built binaries, so the test
    // does not build the release binaries a second time. The release is
    // private to this test, so another test does not rebuild it. The run
    // then uses no installed OTP: `PATH` holds no `erl` or `elixir`, `HOME`
    // is an empty directory, and `PBEAM_OTP_ROOT` is unset.
    let directory = test_directory("mix-build");
    let example = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/plug_hello");
    let release = directory.join("release");
    let status = Command::new("mix")
        .args(["release", "--overwrite", "--path"])
        .arg(&release)
        .current_dir(&example)
        .env("MIX_ENV", "prod")
        .status()
        .expect("mix release runs");
    assert!(status.success(), "the private release builds");
    let out = directory.join("out");
    let clean = directory.join("clean");
    let home = directory.join("home");
    fs::create_dir_all(&clean).unwrap();
    fs::create_dir_all(&home).unwrap();
    let output = out.join("plug_hello");
    let status = Command::new("mix")
        .args(["pbeam.build", "--output"])
        .arg(&output)
        .arg("--release")
        .arg(&release)
        .args([
            "--runtime",
            env!("CARGO_BIN_EXE_pbeam"),
            "--tool",
            env!("CARGO_BIN_EXE_pbeam_bundle"),
        ])
        .current_dir(&example)
        .env("MIX_ENV", "prod")
        .status()
        .expect("mix pbeam.build runs");
    assert!(status.success(), "the task writes the executable");
    let executable = clean.join("plug_hello");
    fs::copy(&output, &executable).unwrap();
    let port = free_port();
    let mut command = Command::new(&executable);
    command
        .current_dir(&clean)
        .env_clear()
        .env("HOME", &home)
        .env("PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if std::env::var_os("LLVM_PROFILE_FILE").is_some() {
        // The coverage build writes its profile file to the working
        // directory. Keep that file out of the clean directory.
        command.env("LLVM_PROFILE_FILE", directory.join("pbeam-%p.profraw"));
    }
    let mut child = command.spawn().expect("the clean run starts");
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stdout_reader = collect(child.stdout.take().unwrap(), Arc::clone(&stdout));
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let stderr_reader = collect(child.stderr.take().unwrap(), Arc::clone(&stderr));
    let stream = wait_for_server(&mut child, port);
    let response = stream.and_then(request);
    let status = stop_with_term(child);
    stdout_reader.join().unwrap();
    stderr_reader.join().unwrap();
    assert!(status.success(), "the clean run exits 0: {status:?}");
    let entries: Vec<_> = fs::read_dir(&clean)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        entries,
        [std::ffi::OsString::from("plug_hello")],
        "the clean run writes no other file next to the executable"
    );
    // The pinned ERTS serves the same release. The status line, the content
    // type, and the body must not change without an installed OTP.
    let erts = erts_release_response(&release);
    fs::remove_dir_all(&directory).unwrap();
    let response = response.unwrap_or_else(|error| {
        let text = |bytes: &Arc<Mutex<Vec<u8>>>| {
            String::from_utf8_lossy(&bytes.lock().unwrap()).into_owned()
        };
        panic!(
            "the clean run on port {port}: {error}\nstatus: {status:?}\nstdout: {}\nstderr: {}",
            text(&stdout),
            text(&stderr)
        )
    });
    assert_eq!(response, erts);
}

/// Run the release of `release` on the pinned ERTS, send `GET /`, and
/// return the response.
fn erts_release_response(release: &std::path::Path) -> (String, String, String) {
    let port = free_port();
    let mut child = without_erl_flags(&mut Command::new(release.join("bin").join("plug_hello")))
        .arg("start")
        .env("PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .expect("the ERTS release starts");
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stdout_reader = collect(child.stdout.take().unwrap(), Arc::clone(&stdout));
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let stderr_reader = collect(child.stderr.take().unwrap(), Arc::clone(&stderr));
    let stream = wait_for_server(&mut child, port);
    let response = stream.and_then(request);
    let status = stop_with_term(child);
    stdout_reader.join().unwrap();
    stderr_reader.join().unwrap();
    assert!(status.success(), "the ERTS release exits 0: {status:?}");
    response.unwrap_or_else(|error| {
        panic!(
            "the ERTS release on port {port}: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&stdout.lock().unwrap()),
            String::from_utf8_lossy(&stderr.lock().unwrap())
        )
    })
}

/// Build `examples/plug_hello` as one file in a clean directory, run it
/// there, and return the response.
fn serve_one_file() -> (String, String, String) {
    let directory = test_directory("one-file");
    let clean = directory.join("clean");
    let temporary = directory.join("tmp");
    fs::create_dir_all(&clean).unwrap();
    fs::create_dir_all(&temporary).unwrap();
    let start = directory.join("start.list");
    let status = Command::new("bash")
        .arg("scripts/example.sh")
        .args(["plug_hello", "--no-build", "--write-start"])
        .arg(&start)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .expect("scripts/example.sh runs");
    assert!(status.success(), "the start list builds");
    let bundle = directory.join("plug_hello.pbeamb");
    let status = Command::new(env!("CARGO_BIN_EXE_pbeam_bundle"))
        .arg("bundle")
        .arg(release_directory())
        .arg(&bundle)
        .args(["elixir", "start_cli"])
        .arg(&start)
        .status()
        .expect("pbeam_bundle runs");
    assert!(status.success(), "the bundle builds");
    let output = clean.join("plug_hello");
    let status = Command::new(env!("CARGO_BIN_EXE_pbeam_bundle"))
        .arg("executable")
        .arg(env!("CARGO_BIN_EXE_pbeam"))
        .arg(&bundle)
        .arg(&output)
        .status()
        .expect("pbeam_bundle runs");
    assert!(status.success(), "the executable builds");
    let port = free_port();
    let mut child = without_erl_flags(&mut Command::new(&output))
        .current_dir(&clean)
        .env("TMPDIR", &temporary)
        .env("PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .expect("the one-file run starts");
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stdout_reader = collect(child.stdout.take().unwrap(), Arc::clone(&stdout));
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let stderr_reader = collect(child.stderr.take().unwrap(), Arc::clone(&stderr));
    let stream = wait_for_server(&mut child, port);
    let response = stream.and_then(request);
    // SIGTERM reaches the boot of the extracted release, because the
    // launcher is the boot. The run removes its extraction at the exit.
    let status = stop_with_term(child);
    stdout_reader.join().unwrap();
    stderr_reader.join().unwrap();
    assert!(
        fs::read_dir(&temporary).unwrap().next().is_none(),
        "the run keeps its extracted directory"
    );
    // The kernel `erl_signal_handler` prints the SIGTERM notice and calls
    // `init:stop/0` (`erl_signal_handler.erl:45-48`). The one-file run gives
    // the same notice and exit status as the same release on the pinned
    // ERTS.
    assert!(
        status.success(),
        "the one-file run exits 0 after SIGTERM: {status:?}"
    );
    let one_file_stdout = String::from_utf8_lossy(&stdout.lock().unwrap()).into_owned();
    let notice = shutdown_notice(&one_file_stdout);
    assert!(
        notice.is_some(),
        "the one-file run writes the shutdown notice: {one_file_stdout}"
    );
    let (erts_status, erts_notice) = shutdown_on_erts();
    assert_eq!(status.code(), erts_status);
    assert_eq!(notice, erts_notice);
    fs::remove_dir_all(&directory).unwrap();
    response.unwrap_or_else(|error| {
        let text = |bytes: &Arc<Mutex<Vec<u8>>>| {
            String::from_utf8_lossy(&bytes.lock().unwrap()).into_owned()
        };
        panic!(
            "the one-file run on port {port}: {error}\nstatus: {status:?}\nstdout: {}\nstderr: {}",
            text(&stdout),
            text(&stderr)
        )
    })
}

/// Send `SIGTERM` to the process group of the run, and wait for the run.
fn stop_with_term(mut child: Child) -> ExitStatus {
    let group = format!("-{}", child.id());
    let _ = Command::new("kill").args(["-TERM", "--", &group]).status();
    child.wait().expect("the child has a status")
}

/// Run the release on the pinned ERTS, wait until its server accepts,
/// send `SIGTERM`, and return the exit status and the SIGTERM notice.
fn shutdown_on_erts() -> (Option<i32>, Option<String>) {
    let port = free_port();
    let mut child = without_erl_flags(&mut Command::new(
        release_directory().join("bin").join("plug_hello"),
    ))
    .arg("start")
    .env("PORT", port.to_string())
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .process_group(0)
    .spawn()
    .expect("the ERTS release starts");
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stdout_reader = collect(child.stdout.take().unwrap(), Arc::clone(&stdout));
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let stderr_reader = collect(child.stderr.take().unwrap(), Arc::clone(&stderr));
    let started = wait_for_server(&mut child, port);
    let status = stop_with_term(child);
    stdout_reader.join().unwrap();
    stderr_reader.join().unwrap();
    started.unwrap_or_else(|error| {
        panic!(
            "the ERTS release on port {port}: {error}\nstderr: {}",
            String::from_utf8_lossy(&stderr.lock().unwrap())
        )
    });
    let stdout = String::from_utf8_lossy(&stdout.lock().unwrap()).into_owned();
    (status.code(), shutdown_notice(&stdout))
}

/// Return the text of the `erl_signal_handler` notice in `output`, without
/// the timestamp that differs between runs.
fn shutdown_notice(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        line.split_once("[notice] ")
            .map(|(_, text)| text.to_owned())
    })
}

/// Return the build directory of `examples/plug_hello`.
fn release_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/examples/plug_hello")
}

/// Return a new directory for one test.
fn test_directory(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "pbeam-serve-{}-{number}-{name}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// Run `scripts/example.sh` with `arguments` on a free port, send `GET /`
/// with `accept-encoding: zstd` and then `GET /` with no `accept-encoding`,
/// return the plain response and the zstd response, and stop the server.
/// `pbeam` sets `PBEAM_BIN` for the pbeam run.
fn serve(arguments: &[&str], pbeam: Option<&str>) -> (Response<String>, Response<Vec<u8>>) {
    let port = free_port();
    let mut command = Command::new("bash");
    command
        .arg("scripts/example.sh")
        .args(arguments)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("PORT", port.to_string())
        .env_remove("PBEAM_BIN")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // The runner starts mix and erlc, so one process group holds the
        // whole run and a kill reaches each process.
        .process_group(0);
    without_erl_flags(&mut command);
    if let Some(pbeam) = pbeam {
        command.env("PBEAM_BIN", pbeam);
    }
    let mut child = command
        .spawn()
        .unwrap_or_else(|error| panic!("scripts/example.sh {arguments:?}: {error}"));
    // Drain both pipes while the run builds, so that a full pipe cannot
    // block the build.
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stdout_reader = collect(child.stdout.take().unwrap(), Arc::clone(&stdout));
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let stderr_reader = collect(child.stderr.take().unwrap(), Arc::clone(&stderr));
    let stream = wait_for_server(&mut child, port);
    let response = stream
        .and_then(|stream| exchange(stream, ZSTD_REQUEST))
        .and_then(|zstd| {
            let stream = TcpStream::connect(("127.0.0.1", port)).map_err(|error| error.to_string());
            Ok((stream.and_then(request)?, zstd))
        });
    let status = stop(child);
    stdout_reader.join().unwrap();
    stderr_reader.join().unwrap();
    let text =
        |bytes: &Arc<Mutex<Vec<u8>>>| String::from_utf8_lossy(&bytes.lock().unwrap()).into_owned();
    let stdout = text(&stdout);
    let stderr = text(&stderr);
    response.unwrap_or_else(|error| {
        panic!(
            "{arguments:?} on port {port}: {error}\nstatus: {status:?}\nstdout: {stdout}\nstderr: {stderr}"
        )
    })
}

/// Read `source` until its end into `output` in a new thread.
fn collect(
    mut source: impl Read + Send + 'static,
    output: Arc<Mutex<Vec<u8>>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0; 4096];
        while let Ok(count @ 1..) = source.read(&mut buffer) {
            output.lock().unwrap().extend_from_slice(&buffer[..count]);
        }
    })
}

/// Return a free port of the loopback address.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("the loopback address has a free port")
        .local_addr()
        .expect("the listener has an address")
        .port()
}

/// Connect to `port` until the server accepts, and return the stream. Give
/// up when the child exits or when [`START_TIME_LIMIT`] passes.
fn wait_for_server(child: &mut Child, port: u16) -> Result<TcpStream, String> {
    let deadline = Instant::now() + START_TIME_LIMIT;
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => return Ok(stream),
            Err(_) if Instant::now() < deadline => {
                if let Some(status) = child.try_wait().expect("the child has a status") {
                    return Err(format!("the example ended with {status}"));
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(format!("no server on port {port}: {error}")),
        }
    }
}

/// Send `GET /` and return the status line, the headers without `date`, one
/// `name: value` line for each in their order, and the body as text.
fn request(stream: TcpStream) -> Result<Response<String>, String> {
    let (status, headers, body) = exchange(stream, PLAIN_REQUEST)?;
    let body = String::from_utf8(body).map_err(|error| error.to_string())?;
    Ok((status, headers, body))
}

/// Send the bytes of `request` and return the status line, the headers
/// without `date`, one `name: value` line for each in their order, and the
/// bytes of the body.
fn exchange(mut stream: TcpStream, request: &[u8]) -> Result<Response<Vec<u8>>, String> {
    stream
        .set_read_timeout(Some(READ_TIME_LIMIT))
        .map_err(|error| error.to_string())?;
    stream
        .write_all(request)
        .map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader
        .read_line(&mut status)
        .map_err(|error| error.to_string())?;
    let status = status.trim_end().to_owned();
    let mut headers = String::new();
    let mut content_length = None;
    loop {
        let mut line = String::new();
        if reader
            .read_line(&mut line)
            .map_err(|error| error.to_string())?
            == 0
        {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if name == "content-length" {
            content_length = value.trim().parse::<usize>().ok();
        }
        if name != "date" {
            headers.push_str(&format!("{name}: {}\n", value.trim()));
        }
    }
    let length = content_length.ok_or("the response has no content-length header")?;
    let mut body = vec![0; length];
    reader
        .read_exact(&mut body)
        .map_err(|error| error.to_string())?;
    Ok((status, headers, body))
}

/// Remove the variables of [`ERL_FLAG_VARIABLES`] from the environment of
/// `command`.
fn without_erl_flags(command: &mut Command) -> &mut Command {
    for name in ERL_FLAG_VARIABLES {
        command.env_remove(name);
    }
    command
}

/// Boot the release of `examples/plug_hello` with its boot script and the
/// `-eval` probe of [`NIF_MODULES`] instead of the `elixir` entry, on the
/// pinned ERTS or on `pbeam erl`, and return the line of the probe: each
/// module with the result of `erlang:module_loaded/1`. The run stops when
/// the probe halts, or after [`START_TIME_LIMIT`].
fn nif_record(pbeam: Option<&str>) -> String {
    let release = fs::canonicalize(release_directory()).unwrap();
    let data = fs::read_to_string(release.join("releases/start_erl.data")).unwrap();
    let (erts, version) = data.trim().split_once(' ').unwrap();
    let bindir = release.join(format!("erts-{erts}/bin"));
    let config = release.join(format!("releases/{version}/sys"));
    let mut command = match pbeam {
        None => Command::new(bindir.join("erl")),
        Some(pbeam) => {
            let mut command = Command::new(pbeam);
            command.arg("erl").arg("--root").arg(&release);
            command.arg("--bindir").arg(&bindir);
            command.arg("--preloaded").arg(preloaded_directory());
            command.arg("--");
            command
        }
    };
    let probe = format!(
        "io:format(\"~w~n\", [[{{M, erlang:module_loaded(M)}} || M <- {NIF_MODULES}]]), halt()."
    );
    command
        .args(["-noshell", "-mode", "embedded", "-config"])
        .arg(&config)
        .arg("-boot")
        .arg(release.join(format!("releases/{version}/start")))
        .args(["-boot_var", "RELEASE_LIB"])
        .arg(release.join("lib"))
        .args(["-eval", &probe])
        .env("RELEASE_ROOT", &release)
        .env("RELEASE_SYS_CONFIG", &config)
        .env("PORT", free_port().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = without_erl_flags(&mut command)
        .spawn()
        .expect("the probe boot starts");
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stdout_reader = collect(child.stdout.take().unwrap(), Arc::clone(&stdout));
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let stderr_reader = collect(child.stderr.take().unwrap(), Arc::clone(&stderr));
    let deadline = Instant::now() + START_TIME_LIMIT;
    let status = loop {
        if let Some(status) = child.try_wait().expect("the child has a status") {
            break Some(status);
        }
        if Instant::now() >= deadline {
            stop(child);
            break None;
        }
        thread::sleep(Duration::from_millis(100));
    };
    stdout_reader.join().unwrap();
    stderr_reader.join().unwrap();
    let stdout = String::from_utf8_lossy(&stdout.lock().unwrap()).into_owned();
    let stderr = String::from_utf8_lossy(&stderr.lock().unwrap()).into_owned();
    let line = stdout.lines().find(|line| line.starts_with("[{"));
    match (status, line) {
        (Some(status), Some(line)) if status.success() => line.to_owned(),
        _ => panic!("the probe boot {pbeam:?}: {status:?}\nstdout: {stdout}\nstderr: {stderr}"),
    }
}

/// Return the `ebin` directory of `erts` of the pinned OTP, which holds the
/// BEAM files of the preloaded modules.
fn preloaded_directory() -> PathBuf {
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-eval",
            "io:put_chars(filename:join(code:lib_dir(erts), \"ebin\")), halt().",
        ])
        .output()
        .expect("erl runs");
    PathBuf::from(String::from_utf8(output.stdout).unwrap())
}

/// Kill the process group of the run, which holds the runner and its build
/// processes, and wait for the runner.
fn stop(mut child: Child) -> ExitStatus {
    let group = format!("-{}", child.id());
    let _ = Command::new("kill").args(["-9", "--", &group]).status();
    child.wait().expect("the child has a status")
}
