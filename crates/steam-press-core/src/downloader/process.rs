//! Cancellation-aware child process execution with streamed output.

use crate::events::{CancellationToken, EventSink, PackEvent};
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::time::Duration;

use super::DownloadError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandOutput {
    pub code: i32,
    pub output: String,
}

pub(crate) fn run_streaming(
    mut command: Command,
    cancel: &CancellationToken,
    sink: &EventSink,
) -> Result<CommandOutput, DownloadError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|error| DownloadError::Failed(format!("could not start the process: {error}")))?;

    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let sink = sink.clone();
        readers.push(std::thread::spawn(move || read_stream(stdout, sink)));
    }
    if let Some(stderr) = child.stderr.take() {
        let sink = sink.clone();
        readers.push(std::thread::spawn(move || read_stream(stderr, sink)));
    }

    let mut cancelled = false;
    let status = loop {
        if cancel.is_cancelled() && !cancelled {
            cancelled = true;
            let _ = child.kill();
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(error) => {
                return Err(DownloadError::Failed(format!(
                    "could not monitor the process: {error}"
                )))
            }
        }
    };

    let mut output = String::new();
    for reader in readers {
        if let Ok(lines) = reader.join() {
            output.push_str(&lines);
        }
    }

    if cancelled {
        return Err(DownloadError::Cancelled);
    }

    Ok(CommandOutput {
        code: status.code().unwrap_or(-1),
        output,
    })
}

fn read_stream(stream: impl Read, sink: EventSink) -> String {
    let reader = BufReader::new(stream);
    let mut output = String::new();
    for line in reader.lines().map_while(Result::ok) {
        sink.emit(PackEvent::Log(line.clone()));
        output.push_str(&line);
        output.push('\n');
    }
    output
}
