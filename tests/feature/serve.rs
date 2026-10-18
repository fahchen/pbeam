//! Serve `GET /` from `examples/plug_hello` on the pinned ERTS and on the
//! `pbeam` binary, and compare the status line, the content type, and the
//! body. The example starts Bandit on the port of the `PORT` variable, so
//! each run gets a free port and the runs do not collide.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::process::CommandExt,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

/// The status line, the content type, and the body of the response of the
/// example to `GET /`.
const RESPONSE: (&str, &str, &str) = (
    "HTTP/1.1 200 OK",
    "text/plain; charset=utf-8",
    "Hello world!",
);

/// The time that the server has to start, and the time that a response has
/// to arrive.
const START_TIME_LIMIT: Duration = Duration::from_secs(120);
const READ_TIME_LIMIT: Duration = Duration::from_secs(60);

#[test]
fn serves_hello_world_from_the_plug_example_as_pinned_erts() {
    // The ERTS run builds the release and runs its entry with `erl +S 1`.
    // The pbeam run uses the same build (`--no-build`), so both runs
    // execute the same compiled modules.
    let erts = serve(&["plug_hello", "--erts"], None);
    let pbeam = serve(
        &["plug_hello", "--no-build"],
        Some(env!("CARGO_BIN_EXE_pbeam")),
    );
    assert_eq!(erts, pbeam);
    assert_eq!(
        pbeam,
        (
            RESPONSE.0.to_owned(),
            RESPONSE.1.to_owned(),
            RESPONSE.2.to_owned(),
        )
    );
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
