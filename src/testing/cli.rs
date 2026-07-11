use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};

use crate::cli::CommandIo;
use crate::support::sync::lock_unpoisoned;

#[derive(Default)]
struct CommandIoState {
    stdout: String,
    stderr: String,
    stdin: VecDeque<String>,
}

/// Capturing CLI I/O fake with queued prompt responses.
#[derive(Clone, Default)]
pub struct CommandIoFake {
    state: Arc<Mutex<CommandIoState>>,
}

impl CommandIoFake {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_input(self, value: impl Into<String>) -> Self {
        self.push_input(value);
        self
    }

    pub fn push_input(&self, value: impl Into<String>) -> &Self {
        lock_unpoisoned(&self.state, "command I/O fake")
            .stdin
            .push_back(value.into());
        self
    }

    pub fn stdout(&self) -> String {
        lock_unpoisoned(&self.state, "command I/O fake")
            .stdout
            .clone()
    }

    pub fn stderr(&self) -> String {
        lock_unpoisoned(&self.state, "command I/O fake")
            .stderr
            .clone()
    }

    pub fn clear(&self) -> &Self {
        let mut state = lock_unpoisoned(&self.state, "command I/O fake");
        state.stdout.clear();
        state.stderr.clear();
        self
    }

    #[track_caller]
    pub fn assert_stdout(&self, expected: &str) -> &Self {
        assert_eq!(self.stdout(), expected);
        self
    }

    #[track_caller]
    pub fn assert_stdout_contains(&self, expected: &str) -> &Self {
        let output = self.stdout();
        assert!(
            output.contains(expected),
            "expected stdout to contain {expected:?}, got {output:?}"
        );
        self
    }

    #[track_caller]
    pub fn assert_stderr(&self, expected: &str) -> &Self {
        assert_eq!(self.stderr(), expected);
        self
    }

    #[track_caller]
    pub fn assert_stderr_contains(&self, expected: &str) -> &Self {
        let output = self.stderr();
        assert!(
            output.contains(expected),
            "expected stderr to contain {expected:?}, got {output:?}"
        );
        self
    }
}

impl CommandIo for CommandIoFake {
    fn write_stdout(&self, message: &str) -> io::Result<()> {
        lock_unpoisoned(&self.state, "command I/O fake")
            .stdout
            .push_str(message);
        Ok(())
    }

    fn write_stderr(&self, message: &str) -> io::Result<()> {
        lock_unpoisoned(&self.state, "command I/O fake")
            .stderr
            .push_str(message);
        Ok(())
    }

    fn read_stdin_line(&self) -> io::Result<String> {
        lock_unpoisoned(&self.state, "command I/O fake")
            .stdin
            .pop_front()
            .map(|value| format!("{value}\n"))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "command I/O fake has no queued input",
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::CommandIo as _;

    use super::CommandIoFake;

    #[test]
    fn fake_captures_streams_and_queues_input() {
        let fake = CommandIoFake::new().with_input("yes");

        fake.write_stdout("out").unwrap();
        fake.write_stderr("err").unwrap();

        assert_eq!(fake.read_stdin_line().unwrap(), "yes\n");
        fake.assert_stdout("out").assert_stderr("err");
    }
}
