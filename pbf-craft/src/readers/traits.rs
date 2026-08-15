use std::rc::Rc;

use crate::models::{Node, Relation, Way};

/// The decoded contents of a single PBF data blob.
///
/// Produced by [`PbfReader::read_next_blob`](crate::readers::PbfReader::read_next_blob) and
/// consumed by the indexed readers. `offset` is the byte offset of the blob in the stream,
/// used for random access.
pub struct BlobData {
    pub nodes: Vec<Node>,
    pub ways: Vec<Way>,
    pub relations: Vec<Relation>,
    pub offset: u64,
}

/// Random access to a PBF stream by blob offset.
///
/// Implemented by [`PbfReader`](crate::readers::PbfReader) and
/// [`CachedReader`](crate::readers::CachedReader) (which adds an in-memory blob cache).
pub trait PbfRandomRead {
    /// Reads and decodes the blob starting at the given byte offset.
    fn read_blob_by_offset(&mut self, offset: u64) -> anyhow::Result<Rc<BlobData>>;
}
