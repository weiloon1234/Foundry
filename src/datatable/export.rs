use std::fmt;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::io::AsyncReadExt as _;

use crate::foundation::{Error, Result};

/// Maximum XLSX size accepted by the compatibility byte-delivery path.
///
/// Deliveries that need larger exports must override
/// [`DatatableExportDelivery::deliver_file`] and stream or copy the artifact.
pub const LEGACY_DATATABLE_EXPORT_MAX_BYTES: u64 = 25 * 1024 * 1024;

const DATATABLE_EXPORT_TEMP_DIR: &str = "foundry-datatable-exports";
const DATATABLE_EXPORT_TEMP_PREFIX: &str = "foundry-datatable-export-";

/// Abstract contract for delivering generated datatable exports.
///
/// Applications must register an implementation (for example, email with an
/// XLSX attachment) via the service registrar before queued exports run.
#[async_trait]
pub trait DatatableExportDelivery: Send + Sync + 'static {
    /// Legacy byte-oriented delivery contract.
    ///
    /// Existing implementations remain supported through the bounded default
    /// [`Self::deliver_file`] adapter. New implementations can override only
    /// `deliver_file` when delivery can consume a file without buffering it.
    async fn deliver(&self, _export: GeneratedDatatableExport, _recipient: &str) -> Result<()> {
        Err(Error::message(
            "datatable export delivery must implement deliver_file or legacy deliver",
        ))
    }

    /// Deliver a completed file-backed XLSX artifact.
    ///
    /// The artifact path remains valid only for this call. The default adapter
    /// checks its file metadata before allocation, reads at most 25 MiB, and
    /// forwards to the legacy byte-oriented method.
    async fn deliver_file(
        &self,
        export: GeneratedDatatableExportFile,
        recipient: &str,
    ) -> Result<()> {
        deliver_file_with_legacy_limit(self, export, recipient, LEGACY_DATATABLE_EXPORT_MAX_BYTES)
            .await
    }
}

/// A generated XLSX export ready for delivery.
pub struct GeneratedDatatableExport {
    pub datatable_id: String,
    pub filename: String,
    pub data: Vec<u8>,
    pub columns: Vec<String>,
}

/// A completed XLSX artifact backed by a temporary file.
///
/// The file is removed when this value is dropped, including unwinding after a
/// delivery panic. Delivery implementations must finish reading or copying it
/// before `deliver_file` returns.
pub struct GeneratedDatatableExportFile {
    datatable_id: String,
    filename: String,
    columns: Vec<String>,
    path: PathBuf,
    size: u64,
}

impl GeneratedDatatableExportFile {
    pub fn datatable_id(&self) -> &str {
        &self.datatable_id
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    /// Open the artifact for bounded streaming/copying by a delivery service.
    pub async fn open(&self) -> Result<tokio::fs::File> {
        tokio::fs::File::open(&self.path).await.map_err(|error| {
            Error::message(format!(
                "failed to open datatable export artifact `{}`: {error}",
                self.path.display()
            ))
        })
    }

    /// Read the artifact only when it fits within an explicit nonzero bound.
    pub async fn read_bounded(&self, max_bytes: u64) -> Result<Vec<u8>> {
        if max_bytes == 0 {
            return Err(Error::message(
                "datatable export byte fallback is disabled because max_bytes is zero",
            ));
        }

        let file = self.open().await?;
        let metadata = file.metadata().await.map_err(|error| {
            Error::message(format!(
                "failed to inspect datatable export artifact `{}`: {error}",
                self.path.display()
            ))
        })?;
        let size = metadata.len();
        if size > max_bytes {
            return Err(legacy_export_too_large_error(size, max_bytes));
        }

        let capacity = usize::try_from(size).map_err(|_| {
            Error::message(format!(
                "datatable export artifact size {size} does not fit in memory on this platform"
            ))
        })?;
        let mut bytes = Vec::with_capacity(capacity);
        file.take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| {
                Error::message(format!(
                    "failed to read datatable export artifact `{}`: {error}",
                    self.path.display()
                ))
            })?;
        if bytes.len() as u64 > max_bytes {
            return Err(legacy_export_too_large_error(bytes.len() as u64, max_bytes));
        }
        Ok(bytes)
    }

    pub(crate) fn create(
        datatable_id: String,
        filename: String,
        columns: Vec<String>,
    ) -> Result<(Self, File)> {
        let directory = std::env::temp_dir().join(DATATABLE_EXPORT_TEMP_DIR);
        std::fs::create_dir_all(&directory).map_err(|error| {
            Error::message(format!(
                "failed to create datatable export temp directory `{}`: {error}",
                directory.display()
            ))
        })?;

        for _ in 0..16 {
            let path = directory.join(format!(
                "{DATATABLE_EXPORT_TEMP_PREFIX}{}.xlsx",
                uuid::Uuid::now_v7()
            ));
            let mut options = OpenOptions::new();
            options.read(true).write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(file) => {
                    return Ok((
                        Self {
                            datatable_id,
                            filename,
                            columns,
                            path,
                            size: 0,
                        },
                        file,
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(Error::message(format!(
                        "failed to create datatable export temp file: {error}"
                    )));
                }
            }
        }

        Err(Error::message(
            "failed to create a unique datatable export temp file",
        ))
    }

    pub(crate) fn refresh_size(mut self) -> Result<Self> {
        self.size = std::fs::metadata(&self.path)
            .map_err(|error| {
                Error::message(format!(
                    "failed to inspect datatable export artifact `{}`: {error}",
                    self.path.display()
                ))
            })?
            .len();
        Ok(self)
    }

    fn buffered(&self, data: Vec<u8>) -> GeneratedDatatableExport {
        GeneratedDatatableExport {
            datatable_id: self.datatable_id.clone(),
            filename: self.filename.clone(),
            data,
            columns: self.columns.clone(),
        }
    }
}

impl fmt::Debug for GeneratedDatatableExportFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GeneratedDatatableExportFile")
            .field("datatable_id", &self.datatable_id)
            .field("filename", &self.filename)
            .field("columns", &self.columns)
            .field("path", &self.path)
            .field("size", &self.size)
            .finish()
    }
}

impl Drop for GeneratedDatatableExportFile {
    fn drop(&mut self) {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(
                    path = %self.path.display(),
                    error = %error,
                    "failed to remove datatable export temp artifact"
                );
            }
        }
    }
}

async fn deliver_file_with_legacy_limit<D>(
    delivery: &D,
    export: GeneratedDatatableExportFile,
    recipient: &str,
    max_bytes: u64,
) -> Result<()>
where
    D: DatatableExportDelivery + ?Sized,
{
    let bytes = export.read_bounded(max_bytes).await?;
    let buffered = export.buffered(bytes);
    delivery.deliver(buffered, recipient).await
}

fn legacy_export_too_large_error(size: u64, max_bytes: u64) -> Error {
    Error::message(format!(
        "datatable export artifact is {size} bytes, exceeding the legacy byte-delivery limit of {max_bytes} bytes; override DatatableExportDelivery::deliver_file to stream or copy the file-backed artifact"
    ))
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::panic::AssertUnwindSafe;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use futures_util::FutureExt as _;
    use tokio::io::AsyncReadExt as _;

    use super::*;

    fn artifact(bytes: &[u8]) -> (GeneratedDatatableExportFile, PathBuf) {
        let (artifact, mut file) = GeneratedDatatableExportFile::create(
            "orders".to_string(),
            "orders.xlsx".to_string(),
            vec!["id".to_string()],
        )
        .unwrap();
        file.write_all(bytes).unwrap();
        file.flush().unwrap();
        drop(file);
        let artifact = artifact.refresh_size().unwrap();
        let path = artifact.path().to_path_buf();
        (artifact, path)
    }

    #[derive(Default)]
    struct FileOnlyDelivery {
        prefixes: Mutex<Vec<[u8; 2]>>,
    }

    #[async_trait]
    impl DatatableExportDelivery for FileOnlyDelivery {
        async fn deliver_file(
            &self,
            export: GeneratedDatatableExportFile,
            _recipient: &str,
        ) -> Result<()> {
            let mut file = export.open().await?;
            let mut prefix = [0; 2];
            file.read_exact(&mut prefix).await.map_err(Error::other)?;
            self.prefixes.lock().unwrap().push(prefix);
            Ok(())
        }
    }

    #[tokio::test]
    async fn file_delivery_bypasses_legacy_bytes_and_cleans_up_after_success() {
        let delivery = FileOnlyDelivery::default();
        let (artifact, path) = artifact(b"PK-streamed");

        delivery
            .deliver_file(artifact, "reports@example.test")
            .await
            .unwrap();

        assert_eq!(
            delivery.prefixes.lock().unwrap().as_slice(),
            &[[b'P', b'K']]
        );
        assert!(!path.exists());
    }

    #[derive(Default)]
    struct LegacyDelivery {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl DatatableExportDelivery for LegacyDelivery {
        async fn deliver(&self, _export: GeneratedDatatableExport, _recipient: &str) -> Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn legacy_delivery_receives_small_file_through_bounded_adapter() {
        let delivery = LegacyDelivery::default();
        let (artifact, path) = artifact(b"PK-legacy");

        delivery
            .deliver_file(artifact, "reports@example.test")
            .await
            .unwrap();

        assert_eq!(delivery.calls.load(Ordering::SeqCst), 1);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn legacy_delivery_rejects_sparse_over_limit_file_before_reading() {
        let (artifact, file) = GeneratedDatatableExportFile::create(
            "orders".to_string(),
            "orders.xlsx".to_string(),
            Vec::new(),
        )
        .unwrap();
        file.set_len(LEGACY_DATATABLE_EXPORT_MAX_BYTES + 1).unwrap();
        drop(file);
        let artifact = artifact.refresh_size().unwrap();
        let path = artifact.path().to_path_buf();
        let delivery = LegacyDelivery::default();

        let error = delivery
            .deliver_file(artifact, "reports@example.test")
            .await
            .unwrap_err();

        assert!(error.to_string().contains(&format!(
            "legacy byte-delivery limit of {} bytes",
            LEGACY_DATATABLE_EXPORT_MAX_BYTES
        )));
        assert_eq!(delivery.calls.load(Ordering::SeqCst), 0);
        assert!(!path.exists());
    }

    struct PanickingFileDelivery;

    #[async_trait]
    impl DatatableExportDelivery for PanickingFileDelivery {
        async fn deliver_file(
            &self,
            _export: GeneratedDatatableExportFile,
            _recipient: &str,
        ) -> Result<()> {
            panic!("delivery panic");
        }
    }

    #[tokio::test]
    async fn file_artifact_is_cleaned_up_when_delivery_panics() {
        let (artifact, path) = artifact(b"PK-panic");

        let result =
            AssertUnwindSafe(PanickingFileDelivery.deliver_file(artifact, "reports@example.test"))
                .catch_unwind()
                .await;

        assert!(result.is_err());
        assert!(!path.exists());
    }
}
