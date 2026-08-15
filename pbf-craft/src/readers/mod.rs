//! Readers for PBF data.
//!
//! - [`crate::readers::PbfReader`] — sequential streaming reader, including parallel filtering
//!   ([`crate::readers::PbfReader::par_find`]) and byte progress reporting ([`crate::readers::PbfReader::progress`]).
//! - [`crate::readers::IterableReader`] — an iterator adapter over `PbfReader` with progress
//!   ([`crate::readers::ReaderProgress`]).
//! - [`crate::readers::IndexedReader`] — random access by element id backed by a `.pif` index, with an
//!   optional in-memory blob cache and dependency resolution.
//! - [`crate::readers::CachedReader`] — a random-access wrapper adding a decoded-blob cache.

mod cached_reader;
mod indexed_reader;
mod iter_reader;
mod raw_reader;
mod traits;

pub use cached_reader::CachedReader;
pub use indexed_reader::IndexedReader;
pub use iter_reader::IterableReader;
pub use raw_reader::{PbfReader, ReaderProgress};
