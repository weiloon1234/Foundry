use std::io::{self, BufRead as _, Write as _};
use std::sync::Arc;

use crate::foundation::{Error, Result};

/// Injectable terminal boundary used by CLI command handlers.
pub trait CommandIo: Send + Sync + 'static {
    fn write_stdout(&self, message: &str) -> io::Result<()>;
    fn write_stderr(&self, message: &str) -> io::Result<()>;
    fn read_stdin_line(&self) -> io::Result<String>;
}

/// Process stdin/stdout/stderr implementation used by normal CLI kernels.
#[derive(Clone, Copy, Debug, Default)]
pub struct TerminalCommandIo;

impl CommandIo for TerminalCommandIo {
    fn write_stdout(&self, message: &str) -> io::Result<()> {
        let mut stdout = io::stdout().lock();
        stdout.write_all(message.as_bytes())?;
        stdout.flush()
    }

    fn write_stderr(&self, message: &str) -> io::Result<()> {
        let mut stderr = io::stderr().lock();
        stderr.write_all(message.as_bytes())?;
        stderr.flush()
    }

    fn read_stdin_line(&self) -> io::Result<String> {
        let mut value = String::new();
        let count = io::stdin().lock().read_line(&mut value)?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "stdin closed while waiting for command input",
            ));
        }
        Ok(value)
    }
}

pub(crate) fn stdout(io: &Arc<dyn CommandIo>, message: &str) -> Result<()> {
    io.write_stdout(message).map_err(Error::other)
}

pub(crate) fn stderr(io: &Arc<dyn CommandIo>, message: &str) -> Result<()> {
    io.write_stderr(message).map_err(Error::other)
}

/// Deterministic, line-oriented progress reporter suitable for terminals and tests.
pub struct CommandProgress {
    io: Arc<dyn CommandIo>,
    label: String,
    current: u64,
    total: u64,
    finished: bool,
}

impl CommandProgress {
    pub(crate) fn start(
        io: Arc<dyn CommandIo>,
        label: impl Into<String>,
        total: u64,
    ) -> Result<Self> {
        let progress = Self {
            io,
            label: label.into(),
            current: 0,
            total,
            finished: false,
        };
        progress.render()?;
        Ok(progress)
    }

    pub fn current(&self) -> u64 {
        self.current
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn advance(&mut self, amount: u64) -> Result<()> {
        self.current = self.current.saturating_add(amount).min(self.total);
        self.render()
    }

    pub fn finish(&mut self) -> Result<()> {
        self.current = self.total;
        self.finished = true;
        self.render()
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    fn render(&self) -> Result<()> {
        stdout(
            &self.io,
            &format!("{}: {}/{}\n", self.label, self.current, self.total),
        )
    }
}
