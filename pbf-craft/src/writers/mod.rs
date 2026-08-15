//! Writers for PBF data.
//!
//! [`crate::writers::PbfWriter`] streams elements into zlib-compressed PBF blobs, with dense-node support
//! and optional bounding-box metadata.

mod raw_writer;

pub use raw_writer::PbfWriter;
