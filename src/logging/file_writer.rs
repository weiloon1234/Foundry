use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;

use crate::support::sync::lock_unpoisoned;
use crate::support::{Clock, Date};

#[derive(Clone)]
pub(crate) struct DateRotatingFileWriter {
    dir: PathBuf,
    clock: Clock,
    retention_days: u32,
    state: Arc<Mutex<DateRotatingState>>,
}

struct DateRotatingState {
    current_date: Date,
    file: File,
}

impl DateRotatingFileWriter {
    pub(crate) fn open(dir: &str, clock: &Clock, retention_days: u32) -> io::Result<Self> {
        let dir = PathBuf::from(dir);
        fs::create_dir_all(&dir)?;

        let today = clock.today();
        let file = open_date_file(&dir, &today)?;

        let writer = Self {
            dir,
            clock: clock.clone(),
            retention_days,
            state: Arc::new(Mutex::new(DateRotatingState {
                current_date: today,
                file,
            })),
        };

        // Run initial cleanup on open
        writer.cleanup_old_logs();

        Ok(writer)
    }

    /// Delete log files older than `retention_days`.
    fn cleanup_old_logs(&self) {
        if self.retention_days == 0 {
            return; // 0 = keep forever
        }

        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        let today = self.clock.today();

        for entry in entries.flatten() {
            let path = entry.path();
            let name = match path.file_stem().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            // Only process .log files with date names (YYYY-MM-DD)
            if path.extension().and_then(|e| e.to_str()) != Some("log") {
                continue;
            }

            // Parse date from filename
            let file_date = match chrono::NaiveDate::parse_from_str(&name, "%Y-%m-%d") {
                Ok(d) => d,
                Err(_) => continue,
            };

            let age_days = (today.as_chrono() - file_date).num_days();

            if age_days > self.retention_days as i64 {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

fn open_date_file(dir: &std::path::Path, date: &Date) -> io::Result<File> {
    let path = dir.join(format!("{date}.log"));
    OpenOptions::new().create(true).append(true).open(path)
}

pub(crate) struct FileWriterGuard<'a> {
    guard: MutexGuard<'a, DateRotatingState>,
}

use std::sync::MutexGuard;

impl<'a> MakeWriter<'a> for DateRotatingFileWriter {
    type Writer = FileWriterGuard<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        let mut state = lock_unpoisoned(&self.state, "log file");

        let today = self.clock.today();
        if today != state.current_date {
            if let Ok(file) = open_date_file(&self.dir, &today) {
                state.file = file;
                state.current_date = today;
                // Clean up old logs on date rotation (once per day)
                self.cleanup_old_logs();
            }
        }

        FileWriterGuard { guard: state }
    }
}

impl<'a> io::Write for FileWriterGuard<'a> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.guard.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.guard.file.flush()
    }
}
