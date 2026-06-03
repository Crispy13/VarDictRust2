//! Shared per-thread BAM IndexedReader cache.
//!
//! IndexedReader owns mutable cursor state, so callers take ownership, fetch the
//! requested region, read locally, and return the reader when done.

use std::cell::RefCell;
use std::collections::HashMap;

use rust_htslib::bam;

thread_local! {
    static INDEXED_READERS: RefCell<HashMap<String, bam::IndexedReader>> =
        RefCell::new(HashMap::new());
}

pub fn take_or_open(bam_path: &str) -> Result<bam::IndexedReader, rust_htslib::errors::Error> {
    if let Some(reader) =
        INDEXED_READERS.with(|cached_readers| cached_readers.borrow_mut().remove(bam_path))
    {
        return Ok(reader);
    }

    bam::IndexedReader::from_path(bam_path)
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
