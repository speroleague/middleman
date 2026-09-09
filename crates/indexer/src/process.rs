//! Process lifetime and stdout bounds, independent of Git output parsing.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("process I/O failed ({0:?})")]
    Io(std::io::ErrorKind),
    #[error("process exceeded its time budget")]
    Timeout,
    #[error("process exceeded its output budget")]
    OutputLimit,
}

pub struct Output {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
}

pub fn run(command: &mut Command, timeout: Duration, max_bytes: usize) -> Result<Output, Error> {
    if timeout.is_zero() {
        return Err(Error::Timeout);
    }
    let started = Instant::now();
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| Error::Io(e.kind()))?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Error::Io(std::io::ErrorKind::BrokenPipe));
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take((max_bytes as u64).saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|e| Error::Io(e.kind()))
            .and({
                if bytes.len() > max_bytes {
                    Err(Error::OutputLimit)
                } else {
                    Ok(bytes)
                }
            });
        let _ = sender.send(result);
    });
    let mut captured = None;
    let outcome = loop {
        if started.elapsed() >= timeout {
            break Err(Error::Timeout);
        }
        if captured.is_none() {
            match receiver.try_recv() {
                Ok(Ok(bytes)) => captured = Some(bytes),
                Ok(Err(error)) => break Err(error),
                Err(mpsc::TryRecvError::Disconnected) => {
                    break Err(Error::Io(std::io::ErrorKind::BrokenPipe));
                }
                Err(mpsc::TryRecvError::Empty) => (),
            }
        }
        match child.try_wait() {
            Ok(Some(status)) if captured.is_some() => {
                break Ok(Output {
                    code: status.code(),
                    stdout: captured.take().unwrap_or_default(),
                });
            }
            Err(error) => break Err(Error::Io(error.kind())),
            _ => thread::sleep(Duration::from_millis(2)),
        }
    };
    if outcome.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    outcome
}
