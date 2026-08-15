use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use super::cached_reader::CachedReader;
use super::raw_reader::PbfReader;
use super::traits::PbfRandomRead;
use crate::models::{BasicElement, Element, ElementType, Node, Relation, Way};
use crate::readers::traits::BlobData;
use crate::utils::file;

/// Version byte of the `.pif` index file format (magic + file size + mtime + entries).
const PIF_MAGIC: u8 = 0x01;

fn get_index_path_from_pbf_path(pbf_path: &str) -> String {
    match pbf_path.rfind('.') {
        Some(dot) => {
            let mut index_path = pbf_path.to_owned();
            index_path.replace_range(dot..pbf_path.len(), ".pif");
            index_path
        }
        None => format!("{}.pif", pbf_path),
    }
}

/// Cheap staleness signature of a PBF file: size + last-modified time (nanoseconds).
///
/// This replaces the previous full-file MD5, which forced a full read of the PBF (minutes on
/// planet-sized files) on every open. A file changed within the same nanosecond while keeping
/// the same size is the only way to fool it; acceptable for an index cache.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PbfFileSignature {
    file_size: u64,
    modified_nanos: i64,
}

impl PbfFileSignature {
    fn of(pbf_file: &str) -> anyhow::Result<Self> {
        let metadata = std::fs::metadata(pbf_file)?;
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let modified_nanos = modified
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        Ok(Self {
            file_size: metadata.len(),
            modified_nanos,
        })
    }
}

struct PbfIndex {
    node_index: BTreeMap<i64, u64>,
    way_index: BTreeMap<i64, u64>,
    relation_index: BTreeMap<i64, u64>,
}

impl PbfIndex {
    pub fn new(pbf_file: &str) -> anyhow::Result<Self> {
        if !pbf_file.ends_with(".pbf") {
            bail!("It's not a .pbf file")
        }

        let index_file_path = get_index_path_from_pbf_path(pbf_file);
        let signature = PbfFileSignature::of(pbf_file)?;

        if file::exists(&index_file_path) {
            // Load the existing index; a stale format, an unreadable file, or a signature
            // mismatch all mean "rebuild".
            if let Ok((index, signature_in_file)) = PbfIndex::load_from_file(&index_file_path) {
                if signature == signature_in_file {
                    return Ok(index);
                }
            }
        }

        let pbf_index = PbfIndex::load_from_pbf_file(pbf_file)?;
        pbf_index.persist(&index_file_path, &signature)?;

        Ok(pbf_index)
    }

    fn load_from_file(index_path: &str) -> anyhow::Result<(PbfIndex, PbfFileSignature)> {
        let mut node_index: BTreeMap<i64, u64> = BTreeMap::new();
        let mut way_index: BTreeMap<i64, u64> = BTreeMap::new();
        let mut relation_index: BTreeMap<i64, u64> = BTreeMap::new();

        let index_file = File::open(index_path)?;
        let mut reader = BufReader::new(index_file);

        // Old-format index files start with a 32-char hex MD5; the new format starts with a
        // magic byte. Anything else is stale and must be rebuilt.
        let magic = reader.read_u8()?;
        if magic != PIF_MAGIC {
            bail!("stale or unsupported index format (magic {:#x})", magic);
        }
        let file_size = reader.read_u64::<LittleEndian>()?;
        let modified_nanos = reader.read_i64::<LittleEndian>()?;
        let signature = PbfFileSignature {
            file_size,
            modified_nanos,
        };

        loop {
            let write_type = reader.read_u8()?;
            if write_type == 0 {
                break;
            }

            let id = reader.read_i64::<LittleEndian>()?;
            let offset = reader.read_u64::<LittleEndian>()?;
            match write_type {
                1 => node_index.insert(id, offset),
                2 => way_index.insert(id, offset),
                3 => relation_index.insert(id, offset),
                _ => bail!("Unsupported write type"),
            };
        }

        Ok((
            PbfIndex {
                node_index,
                way_index,
                relation_index,
            },
            signature,
        ))
    }

    fn load_from_pbf_file(pbf_file_path: &str) -> anyhow::Result<PbfIndex> {
        // Indexing...
        let mut node_index: BTreeMap<i64, u64> = BTreeMap::new();
        let mut way_index: BTreeMap<i64, u64> = BTreeMap::new();
        let mut relation_index: BTreeMap<i64, u64> = BTreeMap::new();

        let mut reader = PbfReader::from_path(pbf_file_path)?;
        // The index maps "last element id per blob" -> blob offset and relies on elements
        // being sorted by id. Track the last seen id of each type across the whole stream and
        // fail loudly on any violation (within a blob or across blobs) instead of silently
        // returning wrong lookup results later.
        let mut last_node_id: Option<i64> = None;
        let mut last_way_id: Option<i64> = None;
        let mut last_relation_id: Option<i64> = None;
        while let Some(blob_data) = reader.read_next_blob()? {
            for node in &blob_data.nodes {
                if let Some(prev) = last_node_id {
                    if node.id < prev {
                        bail!(
                            "PBF file is not sorted by node id (id {} after {}); the index requires sorted input",
                            node.id,
                            prev
                        );
                    }
                }
                last_node_id = Some(node.id);
            }
            for way in &blob_data.ways {
                if let Some(prev) = last_way_id {
                    if way.id < prev {
                        bail!(
                            "PBF file is not sorted by way id (id {} after {}); the index requires sorted input",
                            way.id,
                            prev
                        );
                    }
                }
                last_way_id = Some(way.id);
            }
            for relation in &blob_data.relations {
                if let Some(prev) = last_relation_id {
                    if relation.id < prev {
                        bail!(
                            "PBF file is not sorted by relation id (id {} after {}); the index requires sorted input",
                            relation.id,
                            prev
                        );
                    }
                }
                last_relation_id = Some(relation.id);
            }
            if let Some(last) = blob_data.nodes.last() {
                node_index.insert(last.id, blob_data.offset);
            }
            if let Some(last) = blob_data.ways.last() {
                way_index.insert(last.id, blob_data.offset);
            }
            if let Some(last) = blob_data.relations.last() {
                relation_index.insert(last.id, blob_data.offset);
            }
        }

        let index_instance = PbfIndex {
            node_index,
            way_index,
            relation_index,
        };
        // Indexing completed
        Ok(index_instance)
    }

    pub fn get_offset(&self, element_type: &ElementType, element_id: i64) -> Option<u64> {
        let mut range = match element_type {
            ElementType::Node => self.node_index.range(element_id..),
            ElementType::Way => self.way_index.range(element_id..),
            ElementType::Relation => self.relation_index.range(element_id..),
        };
        range.next().map(|(_, offset)| *offset)
    }

    fn persist(&self, index_path: &str, signature: &PbfFileSignature) -> anyhow::Result<()> {
        // Write to a temp file first so a crash mid-write never leaves a half-written index.
        let tmp_path = format!("{}.tmp", index_path);
        {
            let index_file = File::create(&tmp_path)?;
            let mut writer = BufWriter::new(index_file);
            writer.write_u8(PIF_MAGIC)?;
            writer.write_u64::<LittleEndian>(signature.file_size)?;
            writer.write_i64::<LittleEndian>(signature.modified_nanos)?;
            Self::persist_index_map(&mut writer, &self.node_index, 1)?;
            Self::persist_index_map(&mut writer, &self.way_index, 2)?;
            Self::persist_index_map(&mut writer, &self.relation_index, 3)?;
            // write an end symbol
            writer.write_u8(0)?;
            writer.flush()?;
        }
        std::fs::rename(&tmp_path, index_path)?;
        Ok(())
    }

    fn persist_index_map(
        writer: &mut BufWriter<File>,
        index_map: &BTreeMap<i64, u64>,
        write_type: u8,
    ) -> anyhow::Result<()> {
        for (eid, offset) in index_map.iter() {
            writer.write_u8(write_type)?;
            writer.write_i64::<LittleEndian>(*eid)?;
            writer.write_u64::<LittleEndian>(*offset)?;
        }
        Ok(())
    }
}

/// A reader that provides indexed access to PBF file.
///
/// The `IndexedReader` struct allows for efficient random access to PBF file by using an index.
/// It is generic over a type `T` that implements the `PbfRandomRead` trait, which provides the
/// necessary methods for reading PBF data.
///
/// # Requirements
///
/// The index maps "last element id per blob" to a blob offset, so the PBF file **must be
/// sorted by id within each element type** (the conventional file layout). Unordered files
/// are rejected with an error when the index is built.
///
/// # The `.pif` index file
///
/// On first use a `.pif` index file is created next to the PBF file (a header with a magic
/// byte, the source file's size and mtime, then per-type `(last_id, offset)` entries). The
/// index is reused while the source file's size and mtime are unchanged; it is written
/// atomically via a temporary file.
///
/// # Type Parameters
///
/// * `T` - A type that implements the `PbfRandomRead` trait and provides random access reading of
///   PBF data.
///
/// # Fields
///
/// * `pbf_reader` - An instance of type `T` that is used to read the PBF data.
/// * `pbf_index` - An instance of `PbfIndex` that provides the index for efficient random access.
///
/// # Example
///
/// ```rust
/// use pbf_craft::models::ElementType;
/// use pbf_craft::readers::IndexedReader;
///
/// let mut indexed_reader = IndexedReader::from_path("resources/andorra-latest.osm.pbf").unwrap();
/// let result = indexed_reader.find(&ElementType::Node, 4254529698).unwrap();
/// if let Some(ec) = result {
///    println!("Found element: {:?}", ec);
/// }
/// ```
///
/// If you want to read elements of a PBF file frequently, then the version with caching
/// will make reading more efficient
///
/// ```rust
/// use pbf_craft::models::ElementType;
/// use pbf_craft::readers::IndexedReader;
///
/// let mut indexed_reader = IndexedReader::from_path_with_cache("resources/andorra-latest.osm.pbf", 1000).unwrap();
/// let element_list = indexed_reader.get_with_deps(&ElementType::Way, 1055523837).unwrap();
/// ```
///
pub struct IndexedReader<T: PbfRandomRead> {
    pbf_reader: T,
    pbf_index: PbfIndex,
}

impl IndexedReader<PbfReader<BufReader<File>>> {
    /// Creates a new `IndexedReader` instance from a PBF file.
    pub fn from_path(pbf_file: &str) -> anyhow::Result<IndexedReader<PbfReader<BufReader<File>>>> {
        let pbf_index = PbfIndex::new(pbf_file)?;
        let pbf_reader = PbfReader::from_path(pbf_file)?;
        Ok(IndexedReader {
            pbf_index,
            pbf_reader,
        })
    }
}

impl IndexedReader<CachedReader> {
    /// Creates a new `IndexedReader` instance from a PBF file with a cache.
    ///
    /// # Parameters
    ///
    /// * pbf_file - A path to the PBF file.
    /// * cache_capacity - The number of decoded blobs to keep in memory (entries, not bytes).
    ///   A blob holds about 8000 elements on average, so the resident decoded data is roughly
    ///   `cache_capacity × blob size`; size it against your available memory.
    ///
    pub fn from_path_with_cache(
        pbf_file: &str,
        cache_capacity: usize,
    ) -> anyhow::Result<IndexedReader<CachedReader>> {
        let pbf_index = PbfIndex::new(pbf_file)?;
        let pbf_reader = PbfReader::from_path(pbf_file)?;
        let cached_reader = CachedReader::new(pbf_reader, cache_capacity);
        Ok(IndexedReader {
            pbf_index,
            pbf_reader: cached_reader,
        })
    }
}

impl<T: PbfRandomRead> IndexedReader<T> {
    fn find_element<E, F>(
        &mut self,
        element_type: &ElementType,
        element_id: i64,
        get_vec: F,
    ) -> anyhow::Result<Option<E>>
    where
        E: BasicElement,
        F: FnOnce(&BlobData) -> &Vec<E>,
    {
        let has_offset = self.pbf_index.get_offset(element_type, element_id);
        if has_offset.is_none() {
            return Ok(None);
        }
        let offset = has_offset.unwrap();
        let blob_data = self.pbf_reader.read_blob_by_offset(offset)?;
        let elem = get_vec(&blob_data)
            .iter()
            .find(|e| e.get_id() == element_id);
        match elem {
            Some(e) => Ok(Some(e.clone())),
            None => Ok(None),
        }
    }

    fn find_elements<E, F>(
        &mut self,
        element_type: &ElementType,
        element_ids: &[i64],
        get_vec: F,
    ) -> anyhow::Result<Vec<E>>
    where
        E: BasicElement,
        F: Fn(&BlobData) -> &Vec<E>,
    {
        let id_sets: HashSet<i64> = element_ids.iter().copied().collect();
        let offsets: HashSet<u64> = element_ids
            .iter()
            .filter_map(|id| self.pbf_index.get_offset(element_type, *id))
            .collect();
        let result: Vec<E> = offsets
            .into_iter()
            .map(|offset| {
                let blob_data = self.pbf_reader.read_blob_by_offset(offset)?;
                Ok(get_vec(&blob_data)
                    .iter()
                    .filter(|e| id_sets.contains(&e.get_id()))
                    .cloned()
                    .collect::<Vec<E>>())
            })
            .collect::<anyhow::Result<Vec<Vec<E>>>>()?
            .into_iter()
            .flatten()
            .collect();
        Ok(result)
    }

    /// Finds an node by its ID.
    pub fn find_node(&mut self, node_id: i64) -> anyhow::Result<Option<Node>> {
        self.find_element(&ElementType::Node, node_id, |blob_data| &blob_data.nodes)
    }

    /// Finds nodes by their IDs.
    ///
    /// `find_nodes` is more efficient than calling `find_node` multiple times when you have a batch of node IDs.
    ///
    pub fn find_nodes(&mut self, node_ids: &[i64]) -> anyhow::Result<Vec<Node>> {
        self.find_elements(&ElementType::Node, node_ids, |blob_data| &blob_data.nodes)
    }

    /// Finds a way by its ID.
    pub fn find_way(&mut self, way_id: i64) -> anyhow::Result<Option<Way>> {
        self.find_element(&ElementType::Way, way_id, |blob_data| &blob_data.ways)
    }

    /// Finds ways by their IDs.
    ///
    /// `find_ways` is more efficient than calling `find_way` multiple times when you have a batch of way IDs.
    ///
    pub fn find_ways(&mut self, way_ids: &[i64]) -> anyhow::Result<Vec<Way>> {
        self.find_elements(&ElementType::Way, way_ids, |blob_data| &blob_data.ways)
    }

    /// Finds a relation by its ID.
    pub fn find_relation(&mut self, relation_id: i64) -> anyhow::Result<Option<Relation>> {
        self.find_element(&ElementType::Relation, relation_id, |blob_data| {
            &blob_data.relations
        })
    }

    /// Finds relations by their IDs.
    ///
    /// `find_relations` is more efficient than calling `find_relation` multiple times when you have a batch of relation IDs.
    ///
    pub fn find_relations(&mut self, relation_ids: &[i64]) -> anyhow::Result<Vec<Relation>> {
        self.find_elements(&ElementType::Relation, relation_ids, |blob_data| {
            &blob_data.relations
        })
    }

    /// Finds an element by its type and ID.
    pub fn find(
        &mut self,
        element_type: &ElementType,
        element_id: i64,
    ) -> anyhow::Result<Option<Element>> {
        let target = match element_type {
            ElementType::Node => {
                let t = self.find_node(element_id)?;
                if t.is_none() {
                    return Ok(None);
                }
                Element::Node(t.unwrap())
            }
            ElementType::Way => {
                let t = self.find_way(element_id)?;
                if t.is_none() {
                    return Ok(None);
                }
                Element::Way(t.unwrap())
            }
            ElementType::Relation => {
                let t = self.find_relation(element_id)?;
                if t.is_none() {
                    return Ok(None);
                }
                Element::Relation(t.unwrap())
            }
        };
        Ok(Some(target))
    }

    /// Finds an element with its dependencies.
    ///
    /// When you want to get a Way, this method will also return the Nodes that the Way contains.
    /// When you want to get a Relation, this method will also return the Nodes, Ways, and Relations
    /// that the Relation contains.
    /// So, if you use this method to get a Node, it will be the same as calling `find_node`.
    ///
    /// It is highly recommended to use `IndexedReader::from_path_with_cache` to create an `IndexedReader` instance
    /// when you need to read elements with dependencies frequently.
    ///
    pub fn get_with_deps(
        &mut self,
        element_type: &ElementType,
        element_id: i64,
    ) -> anyhow::Result<Vec<Element>> {
        match element_type {
            ElementType::Node => {
                let node = self.find_node(element_id)?;
                if node.is_none() {
                    return Ok(Vec::with_capacity(0));
                }
                let node = node.unwrap();
                Ok(vec![Element::Node(node)])
            }
            ElementType::Way => self.get_way_with_deps(element_id),
            ElementType::Relation => self.get_relation_with_deps(element_id),
        }
    }

    fn get_way_with_deps(&mut self, way_id: i64) -> anyhow::Result<Vec<Element>> {
        let way = self.find_way(way_id)?;
        if way.is_none() {
            return Ok(Vec::with_capacity(0));
        }
        let way = way.unwrap();
        let node_ids: Vec<i64> = way.way_nodes.iter().map(|way_node| way_node.id).collect();
        let nodes = self.find_nodes(&node_ids)?;

        let mut result: Vec<Element> = vec![Element::Way(way)];
        result.extend(nodes.into_iter().map(Element::Node));
        Ok(result)
    }

    /// Resolves a relation with all its node/way/relation dependencies.
    ///
    /// Member relations are resolved recursively. A `visiting` set tracks the relations on the
    /// current recursion chain: revisiting one (a circular dependency) returns an error instead
    /// of recursing forever / overflowing the stack. Relations reached through different paths
    /// (diamonds) are allowed because each frame removes its own id on the way out.
    fn get_relation_with_deps(&mut self, relation_id: i64) -> anyhow::Result<Vec<Element>> {
        let mut visiting = HashSet::new();
        self.get_relation_with_deps_inner(relation_id, &mut visiting)
    }

    fn get_relation_with_deps_inner(
        &mut self,
        relation_id: i64,
        visiting: &mut HashSet<i64>,
    ) -> anyhow::Result<Vec<Element>> {
        if !visiting.insert(relation_id) {
            bail!(
                "circular relation dependency detected: relation {} is already on the dependency chain",
                relation_id
            );
        }

        let mut result = Vec::new();

        let relation = match self.find_relation(relation_id)? {
            Some(relation) => relation,
            None => {
                // Not on the chain after all; leave the set clean so a later lookup of the
                // same id through another path does not report a false cycle.
                visiting.remove(&relation_id);
                return Ok(Vec::with_capacity(0));
            }
        };
        result.push(Element::Relation(relation.clone()));

        let node_ids: Vec<i64> = relation
            .members
            .iter()
            .filter_map(|member| {
                if member.member_type == ElementType::Node {
                    Some(member.member_id)
                } else {
                    None
                }
            })
            .collect();
        self.find_nodes(node_ids.as_slice())?
            .into_iter()
            .for_each(|node| result.push(Element::Node(node)));

        let way_ids: Vec<i64> = relation
            .members
            .iter()
            .filter_map(|member| {
                if member.member_type == ElementType::Way {
                    Some(member.member_id)
                } else {
                    None
                }
            })
            .collect();
        for way_id in way_ids {
            result.append(&mut self.get_way_with_deps(way_id)?);
        }

        let relation_ids: Vec<i64> = relation
            .members
            .iter()
            .filter_map(|member| {
                if member.member_type == ElementType::Relation {
                    Some(member.member_id)
                } else {
                    None
                }
            })
            .collect();
        for member_relation_id in relation_ids {
            result.append(&mut self.get_relation_with_deps_inner(member_relation_id, visiting)?);
        }

        visiting.remove(&relation_id);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_from_pbf_file() {
        let pbf_file = "./resources/andorra-latest.osm.pbf";
        let index_file = PbfIndex::load_from_pbf_file(pbf_file).unwrap();

        let r1 = index_file.get_offset(&ElementType::Node, 52263877);
        let r2 = index_file.get_offset(&ElementType::Node, 52263878);
        assert_eq!(r1, Some(171));
        assert_eq!(r2, Some(49494));
    }

    #[test]
    fn test_index_from_file() {
        // Build a fresh index in a temp location and verify it persists and reloads with a
        // matching file signature (no full-file checksum, no rebuild on second open).
        let pbf_file = "./resources/andorra-latest.osm.pbf";
        let temp_dir =
            std::env::temp_dir().join(format!("pbf_craft_index_test_{}", std::process::id()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let temp_pbf = temp_dir.join("andorra-latest.osm.pbf");
        std::fs::copy(pbf_file, &temp_pbf).unwrap();
        let pbf_path = temp_pbf.to_str().unwrap();

        let index = PbfIndex::new(pbf_path).unwrap();
        assert_eq!(index.get_offset(&ElementType::Node, 52263877), Some(171));

        let index_path = get_index_path_from_pbf_path(pbf_path);
        let (pbf_index, signature) = PbfIndex::load_from_file(&index_path).unwrap();
        assert_eq!(signature, PbfFileSignature::of(pbf_path).unwrap());
        assert_eq!(
            pbf_index.get_offset(&ElementType::Node, 52263878),
            Some(49494)
        );

        // A second PbfIndex::new must reuse the persisted index (signature match), not rebuild.
        let index2 = PbfIndex::new(pbf_path).unwrap();
        assert_eq!(index2.get_offset(&ElementType::Node, 52263877), Some(171));

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_stale_index_format_is_rebuilt() {
        // The old (pre-2.5) index format starts with a 32-char hex checksum, not the magic
        // byte; such a file must be detected as stale and rebuilt by PbfIndex::new.
        let pbf_file = "./resources/andorra-latest.osm.pbf";
        let temp_dir =
            std::env::temp_dir().join(format!("pbf_craft_stale_index_test_{}", std::process::id()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let temp_pbf = temp_dir.join("andorra-latest.osm.pbf");
        std::fs::copy(pbf_file, &temp_pbf).unwrap();
        let pbf_path = temp_pbf.to_str().unwrap();

        // Write a stale-format index next to the temp pbf.
        let index_path = get_index_path_from_pbf_path(pbf_path);
        let stale = "ba8a2a59183a49c3e624246b8e8138a5".to_string();
        std::fs::write(&index_path, stale).unwrap();

        // load_from_file must refuse the stale format...
        assert!(PbfIndex::load_from_file(&index_path).is_err());

        // ...and PbfIndex::new must transparently rebuild it.
        let index = PbfIndex::new(pbf_path).unwrap();
        assert_eq!(index.get_offset(&ElementType::Node, 52263877), Some(171));

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_index_reader_read() {
        let pbf_file = "./resources/andorra-latest.osm.pbf";
        let mut indexed_reader = IndexedReader::from_path(pbf_file).unwrap();
        let target_op = indexed_reader.find(&ElementType::Node, 4254529698).unwrap();
        let target = target_op.unwrap();
        if let Element::Node(node) = target {
            assert_eq!(node.id, 4254529698);
        } else {
            panic!("Expected Node element");
        }

        let target_op = indexed_reader.find(&ElementType::Way, 1055523837).unwrap();
        let target = target_op.unwrap();
        if let Element::Way(way) = target {
            assert_eq!(way.id, 1055523837);
        } else {
            panic!("Expected Way element");
        }
    }

    #[test]
    fn test_find_nonexistent_element() {
        let pbf_file = "./resources/andorra-latest.osm.pbf";
        let mut indexed_reader = IndexedReader::from_path(pbf_file).unwrap();

        // Test non-existent node
        assert!(indexed_reader.find_node(-1).unwrap().is_none());

        // Test non-existent way
        assert!(indexed_reader.find_way(-1).unwrap().is_none());

        // Test non-existent relation
        assert!(indexed_reader.find_relation(-1).unwrap().is_none());
    }

    #[test]
    fn test_batch_operations() {
        let pbf_file = "./resources/andorra-latest.osm.pbf";
        let mut indexed_reader = IndexedReader::from_path(pbf_file).unwrap();

        // Test find_nodes with mixed existing/non-existing IDs
        let node_ids = vec![4254529698, 4254529699, -1]; // Last ID doesn't exist
        let nodes = indexed_reader.find_nodes(&node_ids).unwrap();
        assert_eq!(nodes.len(), 2);
        assert!(nodes.iter().any(|n| n.id == 4254529698));
        assert!(nodes.iter().any(|n| n.id == 4254529699));

        // Test empty input
        assert!(indexed_reader.find_nodes(&[]).unwrap().is_empty());
    }

    #[test]
    fn test_dependency_resolution() {
        let pbf_file = "./resources/andorra-latest.osm.pbf";
        let mut indexed_reader = IndexedReader::from_path(pbf_file).unwrap();

        // Test getting a way with its nodes
        let way_with_deps = indexed_reader
            .get_with_deps(&ElementType::Way, 1055523837)
            .unwrap();
        assert!(way_with_deps.len() > 1);
        assert!(way_with_deps.iter().any(|e| matches!(e, Element::Way(_))));
        assert!(way_with_deps.iter().any(|e| matches!(e, Element::Node(_))));
    }

    #[test]
    fn test_invalid_file_handling() {
        // Test with non-existent PBF file
        assert!(PbfIndex::new("nonexistent.pbf").is_err());

        // Test with invalid PIF file
        assert!(PbfIndex::load_from_file("nonexistent.pif").is_err());
    }
}
