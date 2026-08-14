use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use super::raw_reader::PbfReader;
use super::raw_reader::ReaderProgress;
use super::traits::BlobData;
use crate::models::{Element, ElementType};

/// A reader that provides an iterable interface for reading PBF data.
///
/// The `IterableReader` struct allows for sequential reading of PBF data by iterating over blobs
/// and elements. It is generic over a type `R` that implements the `Read` and `Send` traits, which
/// provide the necessary methods for reading PBF data from a source.
///
/// # Type Parameters
///
/// * `R` - A type that implements the `Read` and `Send` traits, providing methods for reading PBF data.
///
/// # Example
///
/// ```rust
/// use pbf_craft::models::{Element, ElementType};
/// use pbf_craft::readers::IterableReader;
///
/// let mut reader = IterableReader::from_path("resources/andorra-latest.osm.pbf").unwrap();
/// for element in reader {
///    // Process the element
/// }
/// ```
pub struct IterableReader<R: Read + Send> {
    pbf_reader: PbfReader<R>,
    current_blob: Option<BlobData>,
    current_element_type: ElementType,
    current_element_index: usize,
    /// A read error that occurred between blobs. `Iterator` cannot return `Err`, so the error
    /// is stored here and surfaced (with context) on the next call to `next()`.
    read_error: Option<anyhow::Error>,
}

impl<R: Read + Send> IterableReader<R> {
    /// Creates a new `IterableReader` from a raw pbf reader, reading the first blob eagerly so
    /// a malformed stream fails at construction time.
    pub fn new(mut pbf_reader: PbfReader<R>) -> anyhow::Result<Self> {
        Ok(Self {
            current_blob: pbf_reader.read_next_blob()?,
            current_element_type: ElementType::Node,
            current_element_index: 0,
            read_error: None,
            pbf_reader,
        })
    }

    /// Reports the reader's consumption progress. Delegates to the underlying `PbfReader`,
    /// so `total_bytes` is known when created via `from_path`.
    pub fn progress(&self) -> ReaderProgress {
        self.pbf_reader.progress()
    }

    fn next_element(&mut self) -> Option<Element> {
        loop {
            if let Some(blob) = &self.current_blob {
                if ElementType::Node == self.current_element_type {
                    if self.current_element_index < blob.nodes.len() {
                        let node = blob.nodes.get(self.current_element_index).unwrap();
                        self.current_element_index += 1;
                        return Some(Element::Node(node.clone()));
                    } else {
                        self.current_element_type = ElementType::Way;
                        self.current_element_index = 0;
                    }
                }
                if ElementType::Way == self.current_element_type {
                    if self.current_element_index < blob.ways.len() {
                        let way = blob.ways.get(self.current_element_index).unwrap();
                        self.current_element_index += 1;
                        return Some(Element::Way(way.clone()));
                    } else {
                        self.current_element_type = ElementType::Relation;
                        self.current_element_index = 0;
                    }
                }
                if ElementType::Relation == self.current_element_type {
                    if self.current_element_index < blob.relations.len() {
                        let relation = blob.relations.get(self.current_element_index).unwrap();
                        self.current_element_index += 1;
                        return Some(Element::Relation(relation.clone()));
                    } else {
                        match self.pbf_reader.read_next_blob() {
                            Ok(next) => self.current_blob = next,
                            Err(err) => {
                                self.read_error = Some(err);
                                self.current_blob = None;
                            }
                        }
                        self.current_element_type = ElementType::Node;
                        self.current_element_index = 0;
                    }
                }
            } else {
                return None;
            }
        }
    }
}

impl<R: Read + Send> Iterator for IterableReader<R> {
    type Item = Element;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(err) = &self.read_error {
            panic!("PBF read error during iteration: {}", err);
        }
        self.next_element()
    }
}

impl IterableReader<BufReader<File>> {
    /// Creates a new `IterableReader` from a file path.
    pub fn from_path<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let pbf_reader = PbfReader::from_path(path)?;
        Self::new(pbf_reader)
    }
}
