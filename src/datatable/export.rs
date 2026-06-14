use crate::foundation::Result;
use async_trait::async_trait;

/// Abstract contract for delivering generated datatable exports.
///
/// The framework ships with a no-op implementation. Applications register
/// their own (e.g., email with XLSX attachment) via the service registrar.
#[async_trait]
pub trait DatatableExportDelivery: Send + Sync + 'static {
    async fn deliver(&self, export: GeneratedDatatableExport, recipient: &str) -> Result<()>;
}

/// A generated XLSX export ready for delivery.
pub struct GeneratedDatatableExport {
    pub datatable_id: String,
    pub filename: String,
    pub data: Vec<u8>,
    pub columns: Vec<String>,
}

/// Default no-op delivery implementation.
pub struct NoopExportDelivery;

#[async_trait]
impl DatatableExportDelivery for NoopExportDelivery {
    async fn deliver(&self, _export: GeneratedDatatableExport, _recipient: &str) -> Result<()> {
        // No-op: log or ignore
        Ok(())
    }
}
