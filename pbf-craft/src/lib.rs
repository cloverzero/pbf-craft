//! A pure-Rust library for reading and writing OpenStreetMap **PBF** (Protocolbuffer
//! Binary Format) files.
//!
//! # Modules
//!
//! - [`models`] — the OSM element model: [`models::Node`], [`models::Way`],
//!   [`models::Relation`] and the [`models::Element`] enum, with coordinates in
//!   integer nanodegrees.
//! - [`readers`] — a family of readers for different scenarios:
//!   - [`readers::PbfReader`] — sequential streaming reader (single- or multi-threaded
//!     filtering via [`readers::PbfReader::par_find`]).
//!   - [`readers::IterableReader`] — iterator-based reader with byte progress reporting
//!     ([`readers::ReaderProgress`]).
//!   - [`readers::IndexedReader`] — random access by element id, backed by a `.pif` index
//!     file, with an in-memory blob cache and dependency resolution
//!     ([`readers::IndexedReader::get_with_deps`]).
//! - [`writers`] — [`writers::PbfWriter`], a streaming writer producing zlib-compressed
//!   blobs with dense-node support.
//!
//! # Examples
//!
//! Read PBF data from a file:
//!
//! ```rust
//! use pbf_craft::readers::PbfReader;
//!
//! let mut reader = PbfReader::from_path("resources/andorra-latest.osm.pbf").unwrap();
//! reader.read(|header, element| {
//!     if let Some(header_reader) = header {
//!         // Process header
//!     }
//!     if let Some(element) = element {
//!         // Process element
//!     }
//! }).unwrap();
//! ```
//!
//! Read PBF data with dependencies:
//!
//! ```rust
//! use pbf_craft::models::ElementType;
//! use pbf_craft::readers::IndexedReader;
//!
//! let mut indexed_reader =
//!     IndexedReader::from_path_with_cache("resources/andorra-latest.osm.pbf", 1000).unwrap();
//! let element_list = indexed_reader.get_with_deps(&ElementType::Way, 12345678).unwrap();
//! ```
//!
//! Write PBF data to a file:
//!
//! ```rust
//! use pbf_craft::models::{Element, Node};
//! use pbf_craft::writers::PbfWriter;
//!
//! let mut writer = PbfWriter::from_path(std::env::temp_dir().join("output.osm.pbf"), true).unwrap();
//! writer.write(Element::Node(Node::default())).unwrap();
//! writer.finish().unwrap();
//! ```
//!
//! # Data format notes
//!
//! - **Coordinates**: `Node`/`WayNode`/`Bound` coordinates are i64 **nanodegrees** (the raw
//!   PBF unit; divide by 1e9 for degrees).
//! - **Ordering**: the PBF format does not require sorted elements, but the conventional
//!   layout (all nodes by id, then all ways by id, then all relations by id) is assumed by
//!   `IndexedReader` and most other tools. `PbfWriter` stores elements in the order written —
//!   the caller is responsible for the order.
//! - **Compression**: reading supports `raw`, `zlib`, `lz4` and `zstd` blobs; writing
//!   produces `zlib`-compressed blobs.
//! - **visible flag**: elements default to `visible = true` (per spec, the flag is assumed
//!   true when absent). Elements explicitly marked `visible = false` are written with the
//!   required `HistoricalInformation` feature declared in the header.
//! - **Error handling**: all fallible operations return `anyhow::Result`; malformed or
//!   truncated input surfaces as errors rather than panics.
// Generated protobuf code emits `unused_parens` (a rustc lint) that `#![allow(clippy::all)]`
// inside `mod proto` cannot suppress (parent-module inner attributes do not reach child
// modules). Silence it crate-wide for the generated files.
#![allow(unused_parens)]

mod codecs;
/// Contains models for elements of OpenStreetMap data.
pub mod models;
/// Contains readers for reading PBF data.
pub mod readers;
mod utils;
/// Contains writers for writing PBF data.
pub mod writers;

mod proto {
    #![allow(renamed_and_removed_lints)]
    #![allow(mismatched_lifetime_syntaxes)]
    #![allow(clippy::all)]
    include!(concat!(env!("OUT_DIR"), "/mod.rs"));
}

#[macro_use]
extern crate anyhow;
