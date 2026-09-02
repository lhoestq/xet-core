//! Python bindings for `RangeEditCache`.

use std::sync::Arc;

use pyo3::prelude::*;

use xet_pkg::xet_session::RangeEditCache;

#[pyclass(name = "RangeEditCache")]
pub struct PyRangeEditCache {
    inner: Arc<RangeEditCache>,
}

#[pymethods]
impl PyRangeEditCache {
    #[new]
    fn new() -> Self {
        Self {
            inner: Arc::new(RangeEditCache::new()),
        }
    }

    /// Returns `True` if the cache contains an entry for the given file hash
    /// with a matching file size.
    fn contains(&self, file_hash: &str, file_size: u64) -> bool {
        self.inner.contains(file_hash, file_size)
    }
}