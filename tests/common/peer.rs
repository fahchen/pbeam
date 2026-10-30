//! Start an `epmd` and nodes of the pinned ERTS for the distribution tests.
//!
//! Each [`Epmd`] listens on a free port of its own, and each node of a test
//! finds it through `ERL_EPMD_PORT`, so the tests do not use the `epmd` of
//! the host on port 4369 and do not see each other. The drop of an
//! [`ErtsNode`] kills its `erl` process, and the drop of an [`Epmd`] stops
//! the daemon, also when the test fails.

use std::{
    io::{BufRead, BufReader},
    net::TcpListener,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

/// The time that a test waits for `epmd` and for each line of a node.
const TIMEOUT: Duration = Duration::from_secs(60);

/// An `epmd` daemon of the pinned ERTS on a free port.
pub(crate) struct Epmd {
    /// The port of the daemon, for `ERL_EPMD_PORT`.
    port: String,
}

impl Epmd {
    /// Start `epmd -daemon` on a free port and wait until it answers
    /// `epmd -names`. `-relaxed_command_check` lets the drop stop it while
    /// a node that a test killed is still registered.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let epmd = Epmd::start();
    /// command.env("ERL_EPMD_PORT", epmd.port());
    /// ```
    pub(crate) fn start() -> Epmd {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
            .to_string();
        let epmd = Epmd { port };
        let status = epmd
            .command(&["-daemon", "-relaxed_command_check"])
            .status()
            .unwrap();
        assert!(status.success(), "epmd -daemon: {status}");
        let start = Instant::now();
        while !epmd.command(&["-names"]).output().unwrap().status.success() {
            assert!(start.elapsed() < TIMEOUT, "epmd does not answer");
            thread::sleep(Duration::from_millis(50));
        }
        epmd
    }

    /// Return the port of the daemon.
    pub(crate) fn port(&self) -> &str {
        &self.port
    }

    /// Return an `epmd` command with the arguments `arguments` for the
    /// daemon.
    fn command(&self, arguments: &[&str]) -> Command {
        let mut command = Command::new("epmd");
        command
            .args(arguments)
            .env("ERL_EPMD_PORT", &self.port)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }
}

impl Drop for Epmd {
    fn drop(&mut self) {
        let _ = self.command(&["-kill"]).status();
    }
}

/// A node of the pinned ERTS that runs an expression, and the lines of its
/// standard output.
pub(crate) struct ErtsNode {
    /// The `erl` process.
    child: Child,
    /// The lines of the standard output, from a reader thread.
    lines: Receiver<String>,
}

impl ErtsNode {
    /// Start `erl -noshell` with the `epmd` of `epmd`, the arguments
    /// `arguments`, and `-eval expression`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let node = ErtsNode::start(&epmd, &["-sname", "a"], "io:format(\"~s~n\", [node()]).");
    /// assert!(node.line().starts_with("a@"));
    /// ```
    pub(crate) fn start(epmd: &Epmd, arguments: &[&str], expression: &str) -> ErtsNode {
        let mut child = Command::new("erl")
            .arg("-noshell")
            .args(arguments)
            .args(["-eval", expression])
            .env("ERL_EPMD_PORT", epmd.port())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if sender.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        ErtsNode { child, lines }
    }

    /// Return the next line of the standard output.
    pub(crate) fn line(&self) -> String {
        self.lines
            .recv_timeout(TIMEOUT)
            .expect("the ERTS node writes a line")
    }
}

impl Drop for ErtsNode {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
