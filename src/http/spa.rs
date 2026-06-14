use std::path::PathBuf;

use tower_http::services::{ServeDir, ServeFile};

/// Build a fallback service that serves static files from `dir` and falls back
/// to `dir/index.html` for client-side routing (SPA pattern).
pub(crate) fn spa_fallback(dir: PathBuf) -> ServeDir<ServeFile> {
    let index = dir.join("index.html");
    ServeDir::new(dir).fallback(ServeFile::new(index))
}
