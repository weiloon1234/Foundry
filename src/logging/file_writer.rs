use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing_subscriber::fmt::MakeWriter;

use crate::support::sync::lock_unpoisoned;
use crate::support::{Clock, Date};

#[derive(
    Clone,
    Debug,
    Default,
    Deserialize,
    Serialize,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct LogWriterRuntimeSnapshot {
    pub enabled: bool,
    pub queue_capacity: usize,
    pub pending_records: u64,
    pub accepted_total: u64,
    pub written_total: u64,
    pub dropped_total: u64,
    pub rejected_total: u64,
    pub oversized_total: u64,
    pub write_errors_total: u64,
    pub flush_timeouts_total: u64,
}

#[derive(Default)]
struct WriterStats {
    queue_capacity: usize,
    accepted: AtomicU64,
    written: AtomicU64,
    dropped: AtomicU64,
    rejected: AtomicU64,
    oversized: AtomicU64,
    write_errors: AtomicU64,
    flush_timeouts: AtomicU64,
    pending: AtomicU64,
}

impl WriterStats {
    fn new(queue_capacity: usize) -> Self {
        Self {
            queue_capacity,
            ..Self::default()
        }
    }

    fn snapshot(&self) -> LogWriterRuntimeSnapshot {
        LogWriterRuntimeSnapshot {
            enabled: true,
            queue_capacity: self.queue_capacity,
            pending_records: self.pending.load(Ordering::Relaxed),
            accepted_total: self.accepted.load(Ordering::Relaxed),
            written_total: self.written.load(Ordering::Relaxed),
            dropped_total: self.dropped.load(Ordering::Relaxed),
            rejected_total: self.rejected.load(Ordering::Relaxed),
            oversized_total: self.oversized.load(Ordering::Relaxed),
            write_errors_total: self.write_errors.load(Ordering::Relaxed),
            flush_timeouts_total: self.flush_timeouts.load(Ordering::Relaxed),
        }
    }
}

enum WriterMessage {
    Record(Vec<u8>),
    Flush(SyncSender<io::Result<()>>),
    Shutdown(SyncSender<io::Result<()>>),
}

#[derive(Clone)]
pub(crate) struct BoundedFileWriter {
    sender: SyncSender<WriterMessage>,
    stats: Arc<WriterStats>,
    max_record_bytes: usize,
}

pub(crate) struct FileWriterController {
    sender: SyncSender<WriterMessage>,
    stats: Arc<WriterStats>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

static GLOBAL_FILE_WRITER: OnceLock<Arc<FileWriterController>> = OnceLock::new();

impl BoundedFileWriter {
    pub(crate) fn open(
        dir: &str,
        clock: &Clock,
        retention_days: u32,
        queue_capacity: usize,
        max_record_bytes: usize,
    ) -> io::Result<(Self, Arc<FileWriterController>)> {
        if queue_capacity == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "log writer queue capacity must be greater than zero",
            ));
        }
        if max_record_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "log writer max record bytes must be greater than zero",
            ));
        }

        let sink = DateRotatingFileSink::open(dir, clock, retention_days)?;
        Self::spawn(Box::new(sink), queue_capacity, max_record_bytes)
    }

    fn spawn(
        sink: Box<dyn LogSink>,
        queue_capacity: usize,
        max_record_bytes: usize,
    ) -> io::Result<(Self, Arc<FileWriterController>)> {
        let (sender, receiver) = sync_channel(queue_capacity);
        let stats = Arc::new(WriterStats::new(queue_capacity));
        let worker_stats = stats.clone();
        let worker = std::thread::Builder::new()
            .name("foundry-log-writer".to_string())
            .spawn(move || run_writer(receiver, sink, &worker_stats))?;
        let writer = Self {
            sender: sender.clone(),
            stats: stats.clone(),
            max_record_bytes,
        };
        let controller = Arc::new(FileWriterController {
            sender,
            stats,
            worker: Mutex::new(Some(worker)),
        });
        Ok((writer, controller))
    }

    fn submit(&self, record: Vec<u8>, oversized: bool) {
        if record.is_empty() {
            return;
        }
        if oversized {
            self.stats.oversized.fetch_add(1, Ordering::Relaxed);
            self.stats.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }

        self.stats.pending.fetch_add(1, Ordering::Relaxed);
        match self.sender.try_send(WriterMessage::Record(record)) {
            Ok(()) => {
                self.stats.accepted.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Full(_)) => {
                self.stats.pending.fetch_sub(1, Ordering::Relaxed);
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {
                self.stats.pending.fetch_sub(1, Ordering::Relaxed);
                self.stats.rejected.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

impl FileWriterController {
    pub(crate) fn install_global(self: &Arc<Self>) -> io::Result<()> {
        GLOBAL_FILE_WRITER.set(self.clone()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "global file log writer is already installed",
            )
        })
    }

    pub(crate) fn flush(&self, timeout: Duration) -> io::Result<()> {
        let started = Instant::now();
        let (acknowledge, result) = sync_channel(1);
        self.send_control(WriterMessage::Flush(acknowledge), started, timeout)?;
        receive_control_result(result, timeout.saturating_sub(started.elapsed()))
            .inspect_err(|error| self.record_flush_timeout(error))?
    }

    pub(crate) fn shutdown(&self, timeout: Duration) -> io::Result<()> {
        let started = Instant::now();
        let (acknowledge, result) = sync_channel(1);
        self.send_control(WriterMessage::Shutdown(acknowledge), started, timeout)?;
        let flush_result =
            receive_control_result(result, timeout.saturating_sub(started.elapsed()))
                .inspect_err(|error| self.record_flush_timeout(error))?;
        if let Some(worker) = lock_unpoisoned(&self.worker, "log writer worker").take() {
            worker.join().map_err(|_| {
                io::Error::other("background log writer thread panicked during shutdown")
            })?;
        }
        flush_result
    }

    fn send_control(
        &self,
        mut message: WriterMessage,
        started: Instant,
        timeout: Duration,
    ) -> io::Result<()> {
        loop {
            match self.sender.try_send(message) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Full(returned)) => {
                    if started.elapsed() >= timeout {
                        self.stats.flush_timeouts.fetch_add(1, Ordering::Relaxed);
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "timed out waiting to enqueue log writer control message",
                        ));
                    }
                    message = returned;
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(TrySendError::Disconnected(_)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "background log writer is not running",
                    ));
                }
            }
        }
    }

    fn snapshot(&self) -> LogWriterRuntimeSnapshot {
        self.stats.snapshot()
    }

    fn record_flush_timeout(&self, error: &io::Error) {
        if error.kind() == io::ErrorKind::TimedOut {
            self.stats.flush_timeouts.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub(crate) fn global_snapshot() -> LogWriterRuntimeSnapshot {
    GLOBAL_FILE_WRITER
        .get()
        .map_or_else(LogWriterRuntimeSnapshot::default, |writer| {
            writer.snapshot()
        })
}

pub(crate) async fn flush_global(timeout: Duration) -> crate::foundation::Result<()> {
    let Some(writer) = GLOBAL_FILE_WRITER.get().cloned() else {
        return Ok(());
    };
    crate::support::run_blocking("logging.file_writer.flush", move || {
        writer
            .flush(timeout)
            .map_err(crate::foundation::Error::other)
    })
    .await
}

pub(crate) struct BufferedLogRecord {
    writer: BoundedFileWriter,
    buffer: Vec<u8>,
    oversized: bool,
}

impl BufferedLogRecord {
    fn submit(&mut self) {
        self.writer
            .submit(std::mem::take(&mut self.buffer), self.oversized);
        self.oversized = false;
    }
}

impl Drop for BufferedLogRecord {
    fn drop(&mut self) {
        self.submit();
    }
}

impl<'a> MakeWriter<'a> for BoundedFileWriter {
    type Writer = BufferedLogRecord;

    fn make_writer(&'a self) -> Self::Writer {
        BufferedLogRecord {
            writer: self.clone(),
            buffer: Vec::new(),
            oversized: false,
        }
    }
}

impl Write for BufferedLogRecord {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let remaining = self
            .writer
            .max_record_bytes
            .saturating_sub(self.buffer.len());
        let accepted = remaining.min(buffer.len());
        self.buffer.extend_from_slice(&buffer[..accepted]);
        self.oversized |= accepted < buffer.len();
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.submit();
        Ok(())
    }
}

trait LogSink: Send {
    fn write_record(&mut self, record: &[u8]) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
}

struct DateRotatingFileSink {
    dir: PathBuf,
    clock: Clock,
    retention_days: u32,
    current_date: Date,
    file: File,
}

impl DateRotatingFileSink {
    fn open(dir: &str, clock: &Clock, retention_days: u32) -> io::Result<Self> {
        let dir = PathBuf::from(dir);
        fs::create_dir_all(&dir)?;
        let today = clock.today();
        let file = open_date_file(&dir, &today)?;
        let sink = Self {
            dir,
            clock: clock.clone(),
            retention_days,
            current_date: today,
            file,
        };
        sink.cleanup_old_logs();
        Ok(sink)
    }

    fn rotate_if_needed(&mut self) -> io::Result<()> {
        let today = self.clock.today();
        if today == self.current_date {
            return Ok(());
        }
        self.file = open_date_file(&self.dir, &today)?;
        self.current_date = today;
        self.cleanup_old_logs();
        Ok(())
    }

    fn cleanup_old_logs(&self) {
        if self.retention_days == 0 {
            return;
        }
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let today = self.clock.today();

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("log") {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let Ok(file_date) = chrono::NaiveDate::parse_from_str(name, "%Y-%m-%d") else {
                continue;
            };
            if (today.as_chrono() - file_date).num_days() > self.retention_days as i64 {
                let _ = fs::remove_file(path);
            }
        }
    }
}

impl LogSink for DateRotatingFileSink {
    fn write_record(&mut self, record: &[u8]) -> io::Result<()> {
        self.rotate_if_needed()?;
        self.file.write_all(record)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn open_date_file(dir: &Path, date: &Date) -> io::Result<File> {
    let path = dir.join(format!("{date}.log"));
    OpenOptions::new().create(true).append(true).open(path)
}

fn run_writer(receiver: Receiver<WriterMessage>, mut sink: Box<dyn LogSink>, stats: &WriterStats) {
    while let Ok(message) = receiver.recv() {
        match message {
            WriterMessage::Record(record) => {
                stats.pending.fetch_sub(1, Ordering::Relaxed);
                match sink.write_record(&record) {
                    Ok(()) => {
                        stats.written.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        stats.write_errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            WriterMessage::Flush(acknowledge) => {
                let _ = acknowledge.send(flush_sink(sink.as_mut(), stats));
            }
            WriterMessage::Shutdown(acknowledge) => {
                let _ = acknowledge.send(flush_sink(sink.as_mut(), stats));
                return;
            }
        }
    }
    let _ = flush_sink(sink.as_mut(), stats);
}

fn flush_sink(sink: &mut dyn LogSink, stats: &WriterStats) -> io::Result<()> {
    sink.flush().inspect_err(|_| {
        stats.write_errors.fetch_add(1, Ordering::Relaxed);
    })
}

fn receive_control_result(
    receiver: Receiver<io::Result<()>>,
    timeout: Duration,
) -> io::Result<io::Result<()>> {
    match receiver.recv_timeout(timeout) {
        Ok(result) => Ok(result),
        Err(RecvTimeoutError::Timeout) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "timed out waiting for background log writer",
        )),
        Err(RecvTimeoutError::Disconnected) => Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "background log writer stopped before acknowledging the request",
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::mpsc::sync_channel;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use tracing_subscriber::fmt::MakeWriter;

    use super::{BoundedFileWriter, FileWriterController, LogSink, WriterMessage, WriterStats};
    use crate::support::{Clock, Timezone};

    #[derive(Clone, Default)]
    struct MemorySink {
        records: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl LogSink for MemorySink {
        fn write_record(&mut self, record: &[u8]) -> std::io::Result<()> {
            self.records.lock().unwrap().push(record.to_vec());
            Ok(())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct SlowSink {
        write_delay: Duration,
    }

    impl LogSink for SlowSink {
        fn write_record(&mut self, _record: &[u8]) -> std::io::Result<()> {
            std::thread::sleep(self.write_delay);
            Ok(())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn write_record(writer: &BoundedFileWriter, value: &[u8]) {
        let mut record = writer.make_writer();
        record.write_all(value).unwrap();
        record.flush().unwrap();
    }

    #[test]
    fn bounded_writer_drops_newest_record_when_queue_is_full() {
        let (sender, receiver) = sync_channel(1);
        let stats = Arc::new(WriterStats::new(1));
        let writer = BoundedFileWriter {
            sender,
            stats: stats.clone(),
            max_record_bytes: 64,
        };

        write_record(&writer, b"first");
        write_record(&writer, b"second");

        let WriterMessage::Record(record) = receiver.recv().unwrap() else {
            panic!("expected a queued record");
        };
        assert_eq!(record, b"first");
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.accepted_total, 1);
        assert_eq!(snapshot.dropped_total, 1);
        assert_eq!(snapshot.pending_records, 1);
    }

    #[test]
    fn bounded_writer_drops_oversized_records_without_writing_partial_json() {
        let (sender, receiver) = sync_channel(1);
        let stats = Arc::new(WriterStats::new(1));
        let writer = BoundedFileWriter {
            sender,
            stats: stats.clone(),
            max_record_bytes: 4,
        };

        write_record(&writer, b"oversized");

        assert!(receiver.try_recv().is_err());
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.oversized_total, 1);
        assert_eq!(snapshot.dropped_total, 1);
        assert_eq!(snapshot.accepted_total, 0);
    }

    #[test]
    fn writer_rejects_records_after_receiver_shutdown() {
        let (sender, receiver) = sync_channel(1);
        let stats = Arc::new(WriterStats::new(1));
        let writer = BoundedFileWriter {
            sender,
            stats: stats.clone(),
            max_record_bytes: 64,
        };
        drop(receiver);

        write_record(&writer, b"after-shutdown");

        assert_eq!(stats.snapshot().rejected_total, 1);
        assert_eq!(stats.snapshot().pending_records, 0);
    }

    #[test]
    fn background_writer_drains_concurrent_records_and_flushes_on_shutdown() {
        let sink = MemorySink::default();
        let records = sink.records.clone();
        let (writer, controller) = BoundedFileWriter::spawn(Box::new(sink), 64, 128).unwrap();
        let mut threads = Vec::new();
        for index in 0..32 {
            let writer = writer.clone();
            threads.push(std::thread::spawn(move || {
                write_record(&writer, format!("record-{index}\n").as_bytes());
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }

        controller.shutdown(Duration::from_secs(2)).unwrap();

        let snapshot = controller.snapshot();
        assert_eq!(snapshot.accepted_total, 32);
        assert_eq!(snapshot.written_total, 32);
        assert_eq!(snapshot.pending_records, 0);
        assert_eq!(records.lock().unwrap().len(), 32);
    }

    #[test]
    fn date_rotating_file_writer_flushes_accepted_records_before_shutdown() {
        let directory = tempfile::tempdir().unwrap();
        let clock = Clock::new(Timezone::utc());
        let (writer, controller) =
            BoundedFileWriter::open(directory.path().to_str().unwrap(), &clock, 30, 8, 1_024)
                .unwrap();

        write_record(&writer, b"accepted log record\n");
        controller.shutdown(Duration::from_secs(2)).unwrap();

        let path = directory.path().join(format!("{}.log", clock.today()));
        assert_eq!(std::fs::read(path).unwrap(), b"accepted log record\n");
    }

    #[test]
    fn flush_timeout_is_reported_and_does_not_prevent_later_shutdown() {
        let (writer, controller) = BoundedFileWriter::spawn(
            Box::new(SlowSink {
                write_delay: Duration::from_millis(50),
            }),
            2,
            128,
        )
        .unwrap();
        write_record(&writer, b"slow record\n");

        let error = controller.flush(Duration::from_millis(5)).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert_eq!(controller.snapshot().flush_timeouts_total, 1);

        controller.shutdown(Duration::from_secs(1)).unwrap();
        assert_eq!(controller.snapshot().written_total, 1);
    }

    #[test]
    fn controller_type_remains_send_and_sync_for_global_runtime_use() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<FileWriterController>();
    }
}
