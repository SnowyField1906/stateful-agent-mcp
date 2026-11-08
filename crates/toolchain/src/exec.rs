//! Running configured shell commands with a timeout and bounded output capture.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crate::common::{AgentError, Result};

/// Bytes kept per stream; the rest is read and discarded so the child never blocks.
const MAX_CAPTURE: usize = 4 << 20;
/// How long to wait for the output pipes to close after the command ended.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// What a command produced.
#[derive(Debug)]
pub struct Captured {
    /// Exit code; `None` when killed by a signal or on timeout.
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
    pub elapsed: Duration,
}

impl Captured {
    pub fn success(&self) -> bool {
        self.status == Some(0) && !self.timed_out
    }
}

struct Drain {
    buf: Arc<Mutex<Vec<u8>>>,
    done: mpsc::Receiver<()>,
}

fn drain(mut src: impl Read + Send + 'static) -> Drain {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let sink = Arc::clone(&buf);
    thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        while let Ok(n) = src.read(&mut chunk) {
            if n == 0 {
                break;
            }
            let mut guard = sink.lock().unwrap_or_else(PoisonError::into_inner);
            let room = MAX_CAPTURE.saturating_sub(guard.len());
            guard.extend_from_slice(&chunk[..n.min(room)]);
        }
        let _ = tx.send(());
    });
    Drain { buf, done: rx }
}

impl Drain {
    fn finish(self) -> Vec<u8> {
        let _ = self.done.recv_timeout(DRAIN_GRACE);
        std::mem::take(&mut *self.buf.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

#[cfg(unix)]
fn shell(command: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd
}

#[cfg(not(unix))]
fn shell(command: &str) -> Command {
    let mut cmd = Command::new("cmd");
    cmd.arg("/C").arg(command);
    cmd
}

/// Quote one argument for the platform shell.
#[cfg(unix)]
pub fn quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', r"'\''"))
}

#[cfg(not(unix))]
pub fn quote(arg: &str) -> String {
    format!("\"{}\"", arg.replace('"', "\"\""))
}

#[cfg(unix)]
fn kill_tree(child: &mut Child) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{}", child.id())])
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
}

#[cfg(not(unix))]
fn kill_tree(child: &mut Child) {
    let _ = child.kill();
}

/// Run `command` through the platform shell in `cwd`, feeding `stdin` if given. The command is
/// killed (with its process group on Unix) after `timeout`.
pub fn run_shell(
    command: &str,
    cwd: &Path,
    stdin: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<Captured> {
    let mut cmd = shell(command);
    cmd.current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let started = Instant::now();
    let mut child = cmd
        .spawn()
        .map_err(|e| AgentError::Io(format!("cannot run `{command}`: {e}")))?;
    if let (Some(data), Some(mut pipe)) = (stdin, child.stdin.take()) {
        // Written on its own thread so a command that never reads cannot block us.
        thread::spawn(move || {
            let _ = pipe.write_all(&data);
        });
    }
    let out = child.stdout.take().map(drain);
    let err = child.stderr.take().map(drain);

    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) if started.elapsed() >= timeout => {
                timed_out = true;
                kill_tree(&mut child);
                let _ = child.wait();
                break None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(AgentError::Io(format!("waiting for `{command}`: {e}"))),
        }
    };
    Ok(Captured {
        status,
        stdout: out.map(Drain::finish).unwrap_or_default(),
        stderr: err.map(Drain::finish).unwrap_or_default(),
        timed_out,
        elapsed: started.elapsed(),
    })
}
