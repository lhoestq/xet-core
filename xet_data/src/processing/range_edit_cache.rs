//! Cache support for range-edit uploads.
//!
//! After a range-edit upload, the file's terms + an open subtree (everything but the
//! last term) + the last term's chunks are kept in memory. A subsequent append (or
//! edit of the last term) synthesizes the `file-chunk-hashes` response locally →
//! **zero CAS metadata calls per append**.

use xet_core_structures::merklehash::{ChunkHashList, MerkleHash, MerkleHashSubtree};

/// Maximum number of entries before LRU eviction kicks in.
const MAX_CACHE_ENTRIES: usize = 10;

/// A term (segment) from the original file.
#[derive(Clone, Debug)]
pub struct TermInfo {
    /// Xorb hash (hex string).
    pub xorb_hash: String,
    /// Unpacked byte length of the term.
    pub unpacked_length: u64,
    /// Chunk index range within the xorb, end-exclusive.
    pub chunk_range: (u64, u64),
}

/// A component in the merge sequence of a range-edit upload.
///
/// The merge sequence is: `[gap0, window0, gap1, window1, ..., gapN]`
/// where gaps come from the server's `hashRanges` (untouched regions) and windows
/// come from the uploaded dirty regions.
#[derive(Clone, Debug)]
pub enum MergeComponent {
    /// A gap subtree from CAS (partial merkle state of untouched bytes).
    /// `None` means the gap is empty (adjacent windows or window at file start).
    Gap(Option<MerkleHashSubtree>),
    /// A window subtree from the uploaded dirty region.
    Window {
        /// The chunk hashes and lengths that make up this window.
        chunks: ChunkHashList,
        /// Whether this window starts at byte 0 of the file.
        at_start: bool,
        /// Whether this window extends to the end of the file.
        at_end: bool,
    },
}

/// Payload produced by `upload_ranges()` for cache population.
#[derive(Clone, Debug)]
pub struct RangeEditCachePayload {
    /// File hash (hex) of the new file (after edit).
    pub file_hash: String,
    /// New file size in bytes.
    pub new_size: u64,
    /// Original file size before edit.
    pub original_size: u64,
    /// Original file's terms (segments), in order.
    pub terms: Vec<TermInfo>,
    /// Merge sequence components: `[gap0, window0, gap1, window1, ..., gapN]`.
    pub merge_components: Vec<MergeComponent>,
}

/// Cached merkle state for a previously-range-edited file.
#[derive(Clone, Debug)]
pub struct RangeEditCacheEntry {
    /// Total file size in bytes (must match original_size on reuse).
    pub file_size: u64,
    /// Original file's terms (segments), in order, with verification range hashes.
    pub terms: Vec<TermInfo>,
    /// Partial merkle state of all chunks before the last term.
    /// `None` when the file has a single term.
    pub open_subtree: Option<MerkleHashSubtree>,
    /// The last term's chunks (hash + length).
    pub last_term_chunks: ChunkHashList,
}

/// In-memory cache for range-edit state, keyed by file hash.
///
/// Thread-safe: multiple commits can read/write concurrently.
#[derive(Clone, Default)]
pub struct RangeEditCache {
    inner: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, RangeEditCacheEntry>>>,
}

impl RangeEditCache {
    /// Create a new, empty range-edit cache.
    pub fn new() -> Self {
        Self {
            inner: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// Get a cached entry for the given file hash.
    ///
    /// Returns `None` if the file hash is not in the cache or if the entry's size
    /// doesn't match the provided `file_size`.
    pub fn get(&self, file_hash: &str, file_size: u64) -> Option<RangeEditCacheEntry> {
        let map = self.inner.lock().unwrap();
        map.get(file_hash).and_then(|entry| {
            if entry.file_size == file_size {
                Some(entry.clone())
            } else {
                None
            }
        })
    }

    /// Insert a cache entry. Evicts the oldest entry (FIFO) if the cache is full.
    pub fn insert(&self, file_hash: String, entry: RangeEditCacheEntry) {
        let mut map = self.inner.lock().unwrap();
        map.insert(file_hash, entry);
        while map.len() > MAX_CACHE_ENTRIES {
            if let Some(key) = map.keys().next().cloned() {
                map.remove(&key);
            } else {
                break;
            }
        }
    }

    /// Returns `true` if the cache contains an entry for the given file hash
    /// with a matching file size.
    pub fn contains(&self, file_hash: &str, file_size: u64) -> bool {
        self.get(file_hash, file_size).is_some()
    }
}

/// Build a `RangeEditCacheEntry` from a `RangeEditCachePayload`.
///
/// Returns `None` if the cache entry cannot be built (e.g., the last term is a
/// reused original term whose individual chunks are unknown).
pub fn build_cache_entry_from_payload(
    payload: &RangeEditCachePayload,
) -> Option<RangeEditCacheEntry> {
    let terms = &payload.terms;
    if terms.is_empty() {
        tracing::debug!("build_cache_entry: terms is empty");
        return None;
    }

    let new_size = payload.new_size;
    let last_term = terms.last()?;
    let last_term_size = last_term.unpacked_length;
    let last_term_start = new_size.saturating_sub(last_term_size);

    tracing::debug!("build_cache_entry: new_size={}, last_term_size={}, last_term_start={}", 
        new_size, last_term_size, last_term_start);
    tracing::debug!("build_cache_entry: merge_components count={}", payload.merge_components.len());

    // Check if the trailing gap is empty (required for cache validity).
    let trailing_gap_empty = matches!(
        payload.merge_components.last(),
        Some(MergeComponent::Gap(None))
    );
    if !trailing_gap_empty {
        return None;
    }

    // Build the open_subtree: merge all components before the last term start.
    let mut open_subtree_builder: Option<MerkleHashSubtree> = None;
    let mut last_term_chunks: ChunkHashList = Vec::new();
    let mut at_start = false;
    let mut byte_offset = 0u64;

    for comp in &payload.merge_components {
        match comp {
            MergeComponent::Gap(gap) => {
                if let Some(subtree) = gap {
                    if byte_offset < last_term_start {
                        open_subtree_builder = Some(match open_subtree_builder.take() {
                            Some(prev) => MerkleHashSubtree::merge(&[prev, subtree.clone()]).ok()?,
                            None => subtree.clone(),
                        });
                    }
                }
            }
            MergeComponent::Window {
                chunks,
                at_start: win_at_start,
                at_end,
            } => {
                let window_size: u64 = chunks.iter().map(|(_, l)| *l).sum();
                let window_end = byte_offset + window_size;

                if window_end <= last_term_start {
                    // Entirely before the last term → add to open_subtree
                    at_start = if open_subtree_builder.is_none() {
                        *win_at_start
                    } else {
                        false
                    };
                    open_subtree_builder = Some(MerkleHashSubtree::from_chunks(
                        at_start,
                        chunks,
                        false,
                    ));
                } else if byte_offset < last_term_start {
                    // Window straddles the last term boundary
                    let at_start_for_open = if open_subtree_builder.is_none() {
                        *win_at_start
                    } else {
                        false
                    };

                    // Split the window: take only chunks before the last term start
                    let mut split_offset = last_term_start - byte_offset;
                    let mut open_chunks: ChunkHashList = Vec::new();
                    let mut remaining_chunks: ChunkHashList = Vec::new();
                    let mut cumulative = 0u64;

                    for (hash, len) in chunks {
                        if cumulative + *len <= last_term_start || split_offset <= 0 {
                            open_chunks.push((hash.clone(), *len));
                            split_offset -= len;
                        } else {
                            remaining_chunks.push((hash.clone(), *len));
                        }
                        cumulative += len;
                    }

                    if !open_chunks.is_empty() {
                        open_subtree_builder = Some(MerkleHashSubtree::from_chunks(
                            at_start_for_open,
                            &open_chunks,
                            false,
                        ));
                    }
                    last_term_chunks = remaining_chunks;
                } else {
                    // Entirely at or after the last term
                    if last_term_chunks.is_empty() {
                        last_term_chunks = chunks.clone();
                    }
                }

                byte_offset += window_size;
            }
        }
    }

    if last_term_chunks.is_empty() {
        return None;
    }

    Some(RangeEditCacheEntry {
        file_size: new_size,
        terms: terms.clone(),
        open_subtree: open_subtree_builder,
        last_term_chunks,
    })
}

/// Check if edits are confined to the last term.
///
/// Returns `true` if all edits can be planned purely from the cached state.
pub fn all_edits_confined_to_last_term(
    edits: &[(u64, u64, u64)], // (original_start, original_end, new_length)
    last_term_start_in_original: u64,
) -> bool {
    for &(orig_start, orig_end, _new_length) in edits {
        if orig_start < last_term_start_in_original {
            return false;
        }
    }
    true
}