//! Serve `GET /` from `examples/plug_hello` on the pinned ERTS and on the
//! `pbeam` binary, and compare the status line, the content type, and the
//! body. The one-file run also compares the SIGTERM shutdown notice and the
//! exit status with the pinned ERTS. The example starts Bandit on the port
//! of the `PORT` variable, so each run gets a free port and the runs do not
//! collide.

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

/// The status line, the content type, and the body of the response of the
/// example to `GET /`.
const RESPONSE: (&str, &str, &str) = (
    "HTTP/1.1 200 OK",
    "text/plain; charset=utf-8",
    "Hello world from PBeam!",
);

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
    let erts = serve(&["plug_hello", "--erts"], None);
    let pbeam = serve(
        &["plug_hello", "--no-build"],
        Some(env!("CARGO_BIN_EXE_pbeam")),
    );
    let bundled = serve_one_file();
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
        .args(["plug_hello_run", "main"])
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
    let mut child = Command::new(&output)
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
    let mut child = Command::new(release_directory().join("bin").join("plug_hello"))
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

/// Run `scripts/example.sh` with `arguments` on a free port, send `GET /`,
/// return the status line, the content type, and the body, and stop the
/// server. `pbeam` sets `PBEAM_BIN` for the pbeam run.
fn serve(arguments: &[&str], pbeam: Option<&str>) -> (String, String, String) {
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
    let response = stream.and_then(request);
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

/// Send `GET /` and return the status line, the content type, and the body.
fn request(mut stream: TcpStream) -> Result<(String, String, String), String> {
    stream
        .set_read_timeout(Some(READ_TIME_LIMIT))
        .map_err(|error| error.to_string())?;
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader
        .read_line(&mut status)
        .map_err(|error| error.to_string())?;
    let status = status.trim_end().to_owned();
    let mut content_type = String::new();
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
        match name.trim().to_ascii_lowercase().as_str() {
            "content-type" => content_type = value.trim().to_owned(),
            "content-length" => content_length = value.trim().parse::<usize>().ok(),
            _ => {}
        }
    }
    let length = content_length.ok_or("the response has no content-length header")?;
    let mut body = vec![0; length];
    reader
        .read_exact(&mut body)
        .map_err(|error| error.to_string())?;
    let body = String::from_utf8(body).map_err(|error| error.to_string())?;
    Ok((status, content_type, body))
}

/// Kill the process group of the run, which holds the runner and its build
/// processes, and wait for the runner.
fn stop(mut child: Child) -> ExitStatus {
    let group = format!("-{}", child.id());
    let _ = Command::new("kill").args(["-9", "--", &group]).status();
    child.wait().expect("the child has a status")
}
