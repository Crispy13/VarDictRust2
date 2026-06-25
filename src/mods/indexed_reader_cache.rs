//! Shared per-thread BAM IndexedReader cache.
//!
//! IndexedReader owns mutable cursor state, so callers take ownership, fetch the
//! requested region, read locally, and return the reader when done.

use crate::prelude::HashMap;
use std::cell::RefCell;

use rust_htslib::bam;
use rust_htslib::bam::Read;

/// Size (bytes) of htslib's LRU cache of decompressed BGZF blocks, enabled on
/// each freshly opened IndexedReader. The WES sweep fetches thousands of small,
/// overlapping exon tiles per BAM; adjacent tiles whose first read sits in an
/// already-passed BGZF block force htslib to seek backward and re-inflate those
/// blocks. The cache serves such re-seeks from memory. Pure data-source change:
/// the bytes returned are identical, so pipeline output is byte-for-byte unchanged.
const BGZF_CACHE_BYTES: i32 = 4 * 1024 * 1024;

thread_local! {
    static INDEXED_READERS: RefCell<HashMap<String, bam::IndexedReader>> =
        RefCell::new(HashMap::default());
}

pub fn take_or_open(bam_path: &str) -> Result<bam::IndexedReader, rust_htslib::errors::Error> {
    if let Some(reader) =
        INDEXED_READERS.with(|cached_readers| cached_readers.borrow_mut().remove(bam_path))
    {
        return Ok(reader);
    }

    let reader = bam::IndexedReader::from_path(bam_path)?;
    // SAFETY: `reader` owns a live, non-null htsFile for the duration of the call;
    // hts_set_cache_size only adjusts the BGZF block-cache budget on it.
    unsafe {
        rust_htslib::htslib::hts_set_cache_size(reader.htsfile(), BGZF_CACHE_BYTES);
    }
    Ok(reader)
}

pub fn return_reader(bam_path: String, reader: bam::IndexedReader) {
    INDEXED_READERS.with(|cached_readers| {
        cached_readers.borrow_mut().insert(bam_path, reader);
    });
}

const _: fn() = {
    fn check() {
        fn assert_send<T: Send>() {}

        assert_send::<bam::IndexedReader>();
    }

    check
};
