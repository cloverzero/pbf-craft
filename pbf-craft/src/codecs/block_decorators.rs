use std::collections::HashMap;

use super::field::FieldCodec;
use crate::models::{
    Bound, Element, ElementBase, ElementType, Node, OsmUser, Relation, RelationMember, Tag, Way,
    WayNode,
};
use crate::proto::osmformat;
use crate::proto::osmformat::Relation_MemberType;

/// The PBF required features this crate can decode. Mirrors osmosis's supported set, plus
/// HistoricalInformation because the decoder reads the visible flag.
const SUPPORTED_FEATURES: &[&str] = &["OsmSchema-V0.6", "DenseNodes", "HistoricalInformation"];

pub struct HeaderReader {
    header: osmformat::HeaderBlock,
}

impl HeaderReader {
    pub fn new(header: osmformat::HeaderBlock) -> Self {
        Self { header }
    }

    /// The required features declared by the file's header block.
    pub fn required_features(&self) -> Vec<String> {
        self.header.get_required_features().to_vec()
    }

    /// Validates that every required feature is supported. Called eagerly when the header
    /// block is read so a file we cannot interpret correctly fails loudly instead of silently
    /// returning wrong data.
    pub fn validate_features(&self) -> anyhow::Result<()> {
        let unsupported: Vec<&str> = self
            .header
            .get_required_features()
            .iter()
            .filter(|feature| !SUPPORTED_FEATURES.contains(&feature.as_str()))
            .map(|feature| feature.as_str())
            .collect();
        if !unsupported.is_empty() {
            bail!(
                "PBF file contains unsupported features: {}",
                unsupported.join(", ")
            );
        }
        Ok(())
    }

    /// Pipeline metadata derived from the header (does not validate features; call
    /// `validate_features` on the read path).
    pub fn meta(&self) -> HashMap<String, String> {
        let mut meta: HashMap<String, String> = HashMap::new();
        let optional_features = self.header.get_optional_features();
        if optional_features.contains(&"LocationsOnWays".to_string()) {
            meta.insert("way_node.location_included".to_string(), "true".to_string());
        } else {
            meta.insert(
                "way_node.location_included".to_string(),
                "false".to_string(),
            );
        }
        meta
    }

    pub fn bound(&self) -> Option<Bound> {
        if self.header.has_bbox() {
            let bbox = self.header.get_bbox();
            Some(Bound {
                left: bbox.get_left(),
                right: bbox.get_right(),
                top: bbox.get_top(),
                bottom: bbox.get_bottom(),
                origin: self.header.get_source().to_owned(),
            })
        } else {
            None
        }
    }
}

pub struct PrimitiveReader {
    block: osmformat::PrimitiveBlock,
    decoder: FieldCodec,
}

impl PrimitiveReader {
    pub fn new(block: osmformat::PrimitiveBlock) -> anyhow::Result<Self> {
        Ok(Self {
            decoder: FieldCodec::new_with_block(&block)?,
            block,
        })
    }

    pub fn get_nodes(&self) -> anyhow::Result<Vec<Node>> {
        let mut nodes: Vec<Node> = Vec::new();
        for group in self.block.get_primitivegroup() {
            if group.has_dense() {
                nodes.append(&mut self.process_dense(group.get_dense())?);
            }
            nodes.append(&mut self.process_nodes(group.get_nodes())?);
        }
        Ok(nodes)
    }

    pub fn get_ways(&self) -> anyhow::Result<Vec<Way>> {
        let mut ways: Vec<Way> = Vec::new();
        for group in self.block.get_primitivegroup() {
            ways.append(&mut self.process_ways(group.get_ways())?);
        }
        Ok(ways)
    }

    pub fn get_relations(&self) -> anyhow::Result<Vec<Relation>> {
        let mut relations: Vec<Relation> = Vec::new();
        for group in self.block.get_primitivegroup() {
            relations.append(&mut self.process_relations(group.get_relations())?);
        }
        Ok(relations)
    }

    pub fn get_all_elements(&self) -> anyhow::Result<(Vec<Node>, Vec<Way>, Vec<Relation>)> {
        let mut nodes: Vec<Node> = Vec::new();
        let mut ways: Vec<Way> = Vec::new();
        let mut relations: Vec<Relation> = Vec::new();

        for group in self.block.get_primitivegroup() {
            if group.has_dense() {
                nodes.append(&mut self.process_dense(group.get_dense())?);
            }
            nodes.append(&mut self.process_nodes(group.get_nodes())?);
            ways.append(&mut self.process_ways(group.get_ways())?);
            relations.append(&mut self.process_relations(group.get_relations())?);
        }

        Ok((nodes, ways, relations))
    }

    pub fn for_each_element<F: FnMut(Element)>(&self, mut callback: F) -> anyhow::Result<()> {
        for group in self.block.get_primitivegroup() {
            if group.has_dense() {
                for node in self.process_dense(group.get_dense())? {
                    callback(Element::Node(node));
                }
            }
            for node in self.process_nodes(group.get_nodes())? {
                callback(Element::Node(node));
            }
            for way in self.process_ways(group.get_ways())? {
                callback(Element::Way(way));
            }
            for relation in self.process_relations(group.get_relations())? {
                callback(Element::Relation(relation));
            }
        }
        Ok(())
    }

    fn process_dense(&self, dense: &osmformat::DenseNodes) -> anyhow::Result<Vec<Node>> {
        let node_count = dense.id.len();
        let mut dense_info_iter = DenseInfoIterator::new(dense.get_denseinfo(), node_count);
        let mut id_iter = dense.get_id().iter();
        let mut lat_iter = dense.get_lat().iter();
        let mut lon_iter = dense.get_lon().iter();

        let mut kv_iter = dense.get_keys_vals().iter();

        let mut result = Vec::with_capacity(node_count);
        // DenseInfo timestamp/changeset/uid/user_sid columns are parallel arrays: either the
        // file carries them for every node or for none. Absent columns mean "no metadata", so
        // nodes must read back `timestamp: None` rather than the epoch.
        let has_timestamps = !dense.get_denseinfo().get_timestamp().is_empty();
        let mut node_id: i64 = 0;
        let mut latitude: i64 = 0;
        let mut longitude: i64 = 0;
        loop {
            match (
                id_iter.next(),
                lat_iter.next(),
                lon_iter.next(),
                dense_info_iter.next(),
            ) {
                (Some(id), Some(lat), Some(lon), Some(info)) => {
                    node_id += id;
                    latitude += lat;
                    longitude += lon;
                    let mut node = Node {
                        id: node_id,
                        version: info.version,
                        timestamp: if has_timestamps {
                            Some(self.decoder.decode_timestamp(info.timestamp)?)
                        } else {
                            None
                        },
                        changeset_id: info.changeset,
                        user: if info.uid >= 0 {
                            Some(OsmUser {
                                id: info.uid,
                                name: self.decoder.decode_string(info.user_sid as usize),
                            })
                        } else {
                            // Negative accumulated uid means "no user" (osmosis convention).
                            None
                        },
                        latitude: self.decoder.decode_latitude(latitude)?,
                        longitude: self.decoder.decode_longitude(longitude)?,
                        visible: info.visible,
                        tags: Vec::new(),
                    };

                    loop {
                        let key_index_op = kv_iter.next();
                        let key = match key_index_op {
                            None => break,
                            Some(0) => break,
                            Some(&key_index) => self.decoder.decode_string(key_index as usize),
                        };
                        let value_index_op = kv_iter.next();
                        let value = match value_index_op {
                            None => bail!(
                                "malformed dense nodes: key without corresponding value in keys_vals"
                            ),
                            Some(&value_index) => self.decoder.decode_string(value_index as usize),
                        };
                        node.tags.push(Tag { key, value });
                    }

                    result.push(node);
                }
                (None, None, None, None) => break,
                _ => bail!("malformed dense nodes: id/lat/lon/denseinfo size mismatch"),
            }
        }
        Ok(result)
    }

    fn build_base_element(
        &self,
        id: i64,
        tags: Vec<Tag>,
        info: &osmformat::Info,
    ) -> anyhow::Result<ElementBase> {
        Ok(ElementBase {
            id,
            tags,
            version: info.get_version(),
            // An absent timestamp must stay None, not become the epoch.
            timestamp: if info.has_timestamp() {
                Some(self.decoder.decode_timestamp(info.get_timestamp())?)
            } else {
                None
            },
            changeset_id: info.get_changeset(),
            // A user only exists when both uid and user_sid are present and uid >= 0
            // (osmosis convention; negative or missing uid means "no user").
            user: if info.has_uid() && info.has_user_sid() && info.get_uid() >= 0 {
                Some(OsmUser {
                    id: info.get_uid(),
                    name: self.decoder.decode_string(info.get_user_sid() as usize),
                })
            } else {
                None
            },
            // The visible flag defaults to true when absent (proto2 get_visible() returns
            // false for unset optional fields, which would wrongly mark elements deleted).
            visible: info.has_visible() && info.get_visible(),
        })
    }

    fn process_tags(&self, keys: &[u32], vals: &[u32]) -> anyhow::Result<Vec<Tag>> {
        let mut key_iter = keys.iter();
        let mut val_iter = vals.iter();
        let mut tags: Vec<Tag> = Vec::new();
        loop {
            match (key_iter.next(), val_iter.next()) {
                (Some(&key_index), Some(&val_index)) => {
                    let key = self.decoder.decode_string(key_index as usize);
                    let value = self.decoder.decode_string(val_index as usize);
                    tags.push(Tag { key, value })
                }
                (None, None) => break,
                _ => bail!("malformed primitive: tag key/value count mismatch"),
            }
        }
        Ok(tags)
    }

    fn process_nodes(&self, nodes: &[osmformat::Node]) -> anyhow::Result<Vec<Node>> {
        let mut result = Vec::with_capacity(nodes.len());
        for elm in nodes {
            let tags = self.process_tags(elm.get_keys(), elm.get_vals())?;
            let base_el = if elm.has_info() {
                let info = elm.get_info();
                self.build_base_element(elm.get_id(), tags, info)?
            } else {
                ElementBase::new_with_tags(elm.get_id(), tags)
            };
            let mut node: Node = base_el.into();
            node.latitude = self.decoder.decode_latitude(elm.get_lat())?;
            node.longitude = self.decoder.decode_longitude(elm.get_lon())?;
            result.push(node);
        }
        Ok(result)
    }

    fn process_ways(&self, ways: &[osmformat::Way]) -> anyhow::Result<Vec<Way>> {
        let mut result = Vec::with_capacity(ways.len());
        for elm in ways {
            let tags = self.process_tags(elm.get_keys(), elm.get_vals())?;
            let base_el = if elm.has_info() {
                let info = elm.get_info();
                self.build_base_element(elm.get_id(), tags, info)?
            } else {
                ElementBase::new_with_tags(elm.get_id(), tags)
            };
            let mut way: Way = base_el.into();

            let mut node_id: i64 = 0;
            let mut lat: i64 = 0;
            let mut lon: i64 = 0;
            let mut ref_iter = elm.get_refs().iter();
            let mut lat_iter = elm.get_lat().iter();
            let mut lon_iter = elm.get_lon().iter();
            loop {
                match (ref_iter.next(), lat_iter.next(), lon_iter.next()) {
                    (Some(&ref_delta), Some(&lat_delta), Some(&lon_delta)) => {
                        node_id += ref_delta;
                        lat += lat_delta;
                        lon += lon_delta;
                        way.way_nodes.push(WayNode::new(
                            node_id,
                            self.decoder.decode_latitude(lat)?,
                            self.decoder.decode_longitude(lon)?,
                        ));
                    }
                    (Some(&ref_delta), None, None) => {
                        node_id += ref_delta;
                        way.way_nodes.push(WayNode::new_without_coords(node_id));
                    }
                    (None, None, None) => break,
                    _ => bail!("malformed way: refs/lat/lon size mismatch"),
                }
            }

            result.push(way);
        }
        Ok(result)
    }

    fn process_relations(
        &self,
        relations: &[osmformat::Relation],
    ) -> anyhow::Result<Vec<Relation>> {
        let mut result = Vec::with_capacity(relations.len());
        for elm in relations {
            let tags = self.process_tags(elm.get_keys(), elm.get_vals())?;
            let base_el = if elm.has_info() {
                let info = elm.get_info();
                self.build_base_element(elm.get_id(), tags, info)?
            } else {
                ElementBase::new_with_tags(elm.get_id(), tags)
            };
            let mut relation: Relation = base_el.into();
            relation.members = self.build_relation_members(
                elm.get_memids(),
                elm.get_types(),
                elm.get_roles_sid(),
            )?;
            result.push(relation);
        }
        Ok(result)
    }

    fn build_relation_members(
        &self,
        member_ids: &[i64],
        member_types: &[Relation_MemberType],
        member_roles: &[i32],
    ) -> anyhow::Result<Vec<RelationMember>> {
        let mut mid_iter = member_ids.iter();
        let mut role_iter = member_roles.iter();
        let mut type_iter = member_types.iter();

        let mut result: Vec<RelationMember> = Vec::new();
        let mut member_id: i64 = 0;
        loop {
            match (mid_iter.next(), role_iter.next(), type_iter.next()) {
                (Some(mid), Some(&role), Some(mem_type)) => {
                    member_id += mid;
                    let member_type = match mem_type {
                        Relation_MemberType::NODE => ElementType::Node,
                        Relation_MemberType::WAY => ElementType::Way,
                        Relation_MemberType::RELATION => ElementType::Relation,
                    };
                    let member = RelationMember {
                        member_id,
                        member_type,
                        role: self.decoder.decode_string(role as usize),
                    };
                    result.push(member);
                }
                (None, None, None) => break,
                _ => bail!("malformed relation: memids/types/roles_sid size mismatch"),
            }
        }
        Ok(result)
    }
}

pub struct DenseInfoItem {
    version: i32,
    timestamp: i64,
    changeset: i64,
    uid: i32,
    user_sid: i32,
    visible: bool,
}

pub struct DenseInfoIterator<'a> {
    version_iter: std::slice::Iter<'a, i32>,
    timestamp_iter: std::slice::Iter<'a, i64>,
    changeset_iter: std::slice::Iter<'a, i64>,
    uid_iter: std::slice::Iter<'a, i32>,
    user_sid_iter: std::slice::Iter<'a, i32>,
    visible_iter: std::slice::Iter<'a, bool>,
    remaining: usize,
    timestamp: i64,
    changeset: i64,
    uid: i32,
    user_sid: i32,
}

impl<'a> DenseInfoIterator<'a> {
    /// Iterates over `node_count` dense nodes. Each DenseInfo column is optional: missing
    /// columns fall back to their defaults (version -1, timestamp/changeset/uid/user_sid 0,
    /// visible true — matching osmosis's NOVERSION/NOCHANGESET conventions), so files that
    /// omit a column (e.g. no timestamps) or omit DenseInfo entirely still decode without
    /// error.
    fn new(info: &'a osmformat::DenseInfo, node_count: usize) -> DenseInfoIterator<'a> {
        DenseInfoIterator {
            version_iter: info.get_version().iter(),
            timestamp_iter: info.get_timestamp().iter(),
            changeset_iter: info.get_changeset().iter(),
            uid_iter: info.get_uid().iter(),
            user_sid_iter: info.get_user_sid().iter(),
            visible_iter: info.get_visible().iter(),
            remaining: node_count,
            timestamp: 0,
            changeset: 0,
            uid: 0,
            user_sid: 0,
        }
    }
}

impl<'a> Iterator for DenseInfoIterator<'a> {
    type Item = DenseInfoItem;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        let version = self.version_iter.next().copied().unwrap_or(-1);
        let d_timestamp = self.timestamp_iter.next().copied().unwrap_or(0);
        let d_changeset = self.changeset_iter.next().copied().unwrap_or(0);
        let d_uid = self.uid_iter.next().copied().unwrap_or(0);
        let d_user_sid = self.user_sid_iter.next().copied().unwrap_or(0);
        let visible = self.visible_iter.next().copied().unwrap_or(true);

        self.timestamp += d_timestamp;
        self.changeset += d_changeset;
        self.uid += d_uid;
        self.user_sid += d_user_sid;
        Some(DenseInfoItem {
            version,
            timestamp: self.timestamp,
            changeset: self.changeset,
            uid: self.uid,
            user_sid: self.user_sid,
            visible,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_features_accepts_supported_and_rejects_unknown() {
        let mut header = osmformat::HeaderBlock::new();
        header.required_features.push("OsmSchema-V0.6".to_string());
        header.required_features.push("DenseNodes".to_string());
        assert!(HeaderReader::new(header).validate_features().is_ok());

        let mut header = osmformat::HeaderBlock::new();
        header
            .required_features
            .push("Sort.Type_then_ID".to_string());
        let err = HeaderReader::new(header).validate_features().unwrap_err();
        assert!(err.to_string().contains("Sort.Type_then_ID"));
    }

    #[test]
    fn process_tags_mismatch_is_an_error() {
        let block = osmformat::PrimitiveBlock::new();
        let reader = PrimitiveReader::new(block).unwrap();
        let err = reader.process_tags(&[1, 2], &[1]).unwrap_err();
        assert!(err.to_string().contains("tag key/value count mismatch"));
    }

    #[test]
    fn process_relation_members_mismatch_is_an_error() {
        let block = osmformat::PrimitiveBlock::new();
        let reader = PrimitiveReader::new(block).unwrap();
        let err = reader
            .build_relation_members(&[1, 2], &[Relation_MemberType::NODE], &[0])
            .unwrap_err();
        assert!(err.to_string().contains("size mismatch"));
    }
}
