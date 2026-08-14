use rayon::prelude::*;

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::rc::Rc;

use super::traits::{BlobData, PbfRandomRead};
use crate::codecs::blob::{BlobReader, DecodedBlob};
use crate::codecs::block_decorators::{HeaderReader, PrimitiveReader};
use crate::models::{Element, ElementType};

/// A fundamental reader for PBF data.
///
/// The `PbfReader` struct provides functionality to read and process PBF files,
/// which are commonly used for storing OpenStreetMap (OSM) data. It wraps around
/// a `BlobReader` to handle the low-level reading of blobs from the input source.
///
/// # Type Parameters
///
/// * `R` - A type that implements the `Read` and `Send` traits. This is typically
///   a file or a network stream from which the PBF data is read.
///
/// # Example
///
/// ```rust
/// use pbf_craft::readers::PbfReader;
///
/// let mut reader = PbfReader::from_path("resources/andorra-latest.osm.pbf").unwrap();
/// reader.read(|header, element| {
///     if let Some(header_reader) = header {
///         // Process header
///     }
///     if let Some(element) = element {
///         // Process element
///     }
/// }).unwrap();
/// ```
pub struct PbfReader<R: Read + Send> {
    blob_reader: BlobReader<R>,
}

impl<R: Read + Send> PbfReader<R> {
    /// Creates a new `PbfReader` instance with the specified reader which implements `Read` and `Send` traits.
    pub fn new(reader: R) -> PbfReader<R> {
        Self {
            blob_reader: BlobReader::new(reader),
        }
    }

    /// Reads and decodes the next blob, returning its elements.
    ///
    /// Returns `Ok(None)` at a clean end of stream and `Err` on any malformed or truncated
    /// input (headers are validated for supported required features, blocks for structural
    /// consistency).
    pub fn read_next_blob(&mut self) -> anyhow::Result<Option<BlobData>> {
        if self.blob_reader.eof {
            return Ok(None);
        }
        let offset = self.blob_reader.offset;
        match self.blob_reader.next_blob()? {
            Some(blob) => {
                let data = match blob.decode()? {
                    Some(DecodedBlob::OsmHeader(header)) => {
                        HeaderReader::new(header).validate_features()?;
                        BlobData {
                            nodes: Vec::with_capacity(0),
                            ways: Vec::with_capacity(0),
                            relations: Vec::with_capacity(0),
                            offset,
                        }
                    }
                    Some(DecodedBlob::OsmData(data)) => {
                        let decorator = PrimitiveReader::new(data)?;
                        let (nodes, ways, relations) = decorator.get_all_elements()?;
                        BlobData {
                            nodes,
                            ways,
                            relations,
                            offset,
                        }
                    }
                    None => BlobData {
                        nodes: Vec::with_capacity(0),
                        ways: Vec::with_capacity(0),
                        relations: Vec::with_capacity(0),
                        offset,
                    },
                };
                Ok(Some(data))
            }
            None => Ok(None),
        }
    }

    /// Reads and processes header and elements using the provided callback function.
    ///
    /// This is a single-threaded method where all elements are iterated over one by one
    /// in the order recorded in the PBF file.
    ///
    /// # Arguments
    ///
    /// * `callback` - A mutable closure that takes two optional arguments:
    ///     - `Option<HeaderReader>`: Some if a header is decoded, None otherwise.
    ///     - `Option<Element>`: Some if an element is decoded, None otherwise.
    ///
    /// # Returns
    ///
    /// * `anyhow::Result<()>` - Returns an Ok result if all blobs are processed successfully,
    ///   or an error if any blob decoding fails (including unsupported required features).
    ///
    /// # Errors
    ///
    /// This function will return an error if any PBF decoding fails.
    ///
    /// # Example
    ///
    /// ```rust
    /// use pbf_craft::readers::PbfReader;
    ///
    /// let mut reader = PbfReader::from_path("resources/andorra-latest.osm.pbf").unwrap();
    /// reader.read(|header, element| {
    ///     if let Some(header_reader) = header {
    ///         // Process header
    ///     }
    ///     if let Some(element) = element {
    ///         // Process element
    ///     }
    /// }).unwrap();
    /// ```
    pub fn read<F>(&mut self, mut callback: F) -> anyhow::Result<()>
    where
        F: FnMut(Option<HeaderReader>, Option<Element>),
    {
        while let Some(blob) = self.blob_reader.next_blob()? {
            match blob.decode()? {
                Some(DecodedBlob::OsmHeader(b)) => {
                    let header_reader = HeaderReader::new(b);
                    header_reader.validate_features()?;
                    callback(Some(header_reader), None);
                }
                Some(DecodedBlob::OsmData(data)) => {
                    let decorator = PrimitiveReader::new(data)?;
                    decorator.for_each_element(|el| callback(None, Some(el)))?;
                }
                None => {}
            }
        }
        Ok(())
    }

    /// Finds elements in parallel.
    ///
    /// # Arguments
    ///
    /// * `inclination` - An optional reference to an `ElementType` that specifies the type of elements to find.
    ///   If `None`, all element types are considered.
    /// * `callback` - A closure that takes a reference to an `Element` and returns a boolean indicating
    ///   whether the element should be included in the result. The closure must be `Send` and `Sync`.
    ///
    /// # Returns
    ///
    /// * `anyhow::Result<Vec<Element>>` - Returns a vector of elements that match the criteria specified
    ///   by the callback function. If an error occurs during PBF decoding, an error is returned.
    ///
    /// # Errors
    ///
    /// This function will return an error if any PBF decoding fails.
    ///
    /// # Example
    ///
    /// ```rust
    /// use pbf_craft::models::ElementType;
    /// use pbf_craft::readers::PbfReader;
    ///
    /// let mut reader = PbfReader::from_path("resources/andorra-latest.osm.pbf").unwrap();
    /// let elements = reader.par_find(Some(&ElementType::Node), |element| {
    ///     // Filter logic for nodes
    ///     true
    /// }).unwrap();
    /// ```
    pub fn par_find<F>(
        self,
        inclination: Option<&ElementType>,
        callback: F,
    ) -> anyhow::Result<Vec<Element>>
    where
        F: Fn(&Element) -> bool + Send + Sync,
    {
        // Single streaming pipeline: `par_bridge` pulls one blob per worker at a time, each
        // blob is decoded, filtered and merged incrementally, so memory stays bounded by the
        // worker count plus the final result — collecting all decoded blocks up front would
        // hold the whole (planet-sized) file's uncompressed data in memory. Decode errors
        // flow through the Result items and the reduce instead of panicking.
        let result = self
            .blob_reader
            .par_bridge()
            .map(|blob| -> anyhow::Result<Vec<Element>> {
                let decoded = match blob?.decode()? {
                    Some(DecodedBlob::OsmData(b)) => Some(PrimitiveReader::new(b)?),
                    _ => None,
                };
                let Some(p) = decoded else {
                    return Ok(Vec::new());
                };
                if let Some(element_type) = inclination {
                    let result = match element_type {
                        ElementType::Node => p
                            .get_nodes()?
                            .into_iter()
                            .map(Element::Node)
                            .filter(&callback)
                            .collect::<Vec<Element>>(),
                        ElementType::Way => p
                            .get_ways()?
                            .into_iter()
                            .map(Element::Way)
                            .filter(&callback)
                            .collect::<Vec<Element>>(),
                        ElementType::Relation => p
                            .get_relations()?
                            .into_iter()
                            .map(Element::Relation)
                            .filter(&callback)
                            .collect::<Vec<Element>>(),
                    };
                    Ok(result)
                } else {
                    let (nodes, ways, relations) = p.get_all_elements()?;
                    let mut result: Vec<Element> = nodes
                        .into_iter()
                        .map(Element::Node)
                        .filter(&callback)
                        .collect();
                    result.extend(ways.into_iter().map(Element::Way).filter(&callback));
                    result.extend(
                        relations
                            .into_iter()
                            .map(Element::Relation)
                            .filter(&callback),
                    );
                    Ok(result)
                }
            })
            .reduce(
                || Ok(Vec::new()),
                |acc: anyhow::Result<Vec<Element>>, item: anyhow::Result<Vec<Element>>| match (
                    acc, item,
                ) {
                    (Ok(mut a), Ok(b)) => {
                        a.extend(b);
                        Ok(a)
                    }
                    (Err(e), _) | (_, Err(e)) => Err(e),
                },
            )?;
        Ok(result)
    }
}

impl PbfReader<BufReader<File>> {
    /// Creates a new `PbfReader` instance with the specified file path.
    pub fn from_path<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let f = File::open(path)?;
        let reader = BufReader::new(f);
        Ok(Self::new(reader))
    }

    /// Rewinds the reader to the beginning of the file.
    pub fn rewind(&mut self) -> anyhow::Result<()> {
        self.blob_reader.rewind()
    }
}

impl PbfRandomRead for PbfReader<BufReader<File>> {
    fn read_blob_by_offset(&mut self, offset: u64) -> anyhow::Result<Rc<BlobData>> {
        self.blob_reader.seek(offset)?;
        let data = self
            .read_next_blob()?
            .ok_or(anyhow!("no blob data found."))?;
        Ok(Rc::new(data))
    }
}
