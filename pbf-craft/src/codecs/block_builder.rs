use std::collections::HashMap;

use protobuf::RepeatedField;

use super::field::FieldCodec;
use crate::models::{Element, ElementType, Node, Relation, Tag, Way};
use crate::proto::osmformat;

struct StringTableBuilder {
    strings: Vec<String>,
    id_map: HashMap<String, usize>,
}

impl StringTableBuilder {
    /// Creates an empty string table with the reserved index 0 pre-populated.
    ///
    /// The PBF spec reserves string-table index 0 as the dense-format tag terminator, so
    /// entry 0 must ALWAYS be the blank string. `add` therefore returns 0 for `""` without
    /// inserting a duplicate entry, and no other string can ever occupy index 0.
    pub fn new() -> Self {
        let strings = vec![String::new()];
        let mut id_map = HashMap::new();
        id_map.insert(String::new(), 0);
        Self { strings, id_map }
    }
    pub fn add(&mut self, string: String) -> i32 {
        if let Some(id) = self.id_map.get(&string) {
            return *id as i32;
        }
        self.strings.push(string.clone());
        let id = self.strings.len() - 1;
        self.id_map.insert(string, id);
        id as i32
    }

    pub fn into_string_table(self) -> osmformat::StringTable {
        let string_bytes: Vec<Vec<u8>> = self
            .strings
            .into_iter()
            .map(|string| string.as_bytes().to_vec())
            .collect();
        let mut string_table = osmformat::StringTable::new();
        string_table.set_s(RepeatedField::from_vec(string_bytes));
        string_table
    }
}

pub struct PrimitiveBuilder {
    block: osmformat::PrimitiveBlock,
    codec: FieldCodec,
    string_table: StringTableBuilder,
}

impl PrimitiveBuilder {
    pub fn new() -> Self {
        let block = osmformat::PrimitiveBlock::new();
        Self {
            codec: FieldCodec::new(block.get_granularity(), block.get_date_granularity()),
            block,
            string_table: StringTableBuilder::new(),
        }
    }

    fn encode_dense_nodes(&mut self, nodes: Vec<Node>) -> osmformat::DenseNodes {
        let mut dense_info = osmformat::DenseInfo::new();
        let mut dense = osmformat::DenseNodes::new();

        let mut previous_id = 0;
        let mut previous_lat = self.codec.encode_latitude(0);
        let mut previous_lon = self.codec.encode_latitude(0);
        let mut previous_changeset = 0;
        let mut previous_timestamp = 0;
        let mut previous_uid = 0;
        let mut previous_sid = 0;

        // The DenseInfo timestamp column is a parallel array: it must cover every node or
        // none. `build` has already fallen back to sparse encoding for blocks with mixed
        // timestamp presence, so here presence is uniform: emit the column only when every
        // node carries a timestamp, otherwise omit it entirely so the read side reports
        // `timestamp: None` instead of inventing the epoch.
        let write_timestamps = nodes.iter().all(|node| node.timestamp.is_some());

        for node in nodes {
            dense.id.push(node.id - previous_id);

            let lat = self.codec.encode_latitude(node.latitude);
            let lon = self.codec.encode_longitude(node.longitude);
            dense.lat.push(lat - previous_lat);
            dense.lon.push(lon - previous_lon);

            dense_info
                .changeset
                .push(node.changeset_id - previous_changeset);
            dense_info.version.push(node.version);
            dense_info.visible.push(node.visible);

            if write_timestamps {
                let tt = self
                    .codec
                    .encode_timestamp(node.timestamp.expect("checked above"));
                dense_info.timestamp.push(tt - previous_timestamp);
                previous_timestamp = tt;
            }

            (previous_uid, previous_sid) = if let Some(user) = node.user {
                dense_info.uid.push(user.id - previous_uid);
                let user_sid = self.string_table.add(user.name);
                dense_info.user_sid.push(user_sid - previous_sid);
                (user.id, user_sid)
            } else {
                // No user: accumulate the uid to -1 (osmosis convention) so readers map the
                // accumulated value to "no user" instead of a phantom uid-0 user.
                dense_info.uid.push(-1 - previous_uid);
                let user_sid = self.string_table.add("".to_string());
                dense_info.user_sid.push(user_sid - previous_sid);
                (-1, user_sid)
            };

            for tag in node.tags {
                dense.keys_vals.push(self.string_table.add(tag.key));
                dense.keys_vals.push(self.string_table.add(tag.value));
            }
            dense.keys_vals.push(0);

            previous_id = node.id;
            previous_lat = lat;
            previous_lon = lon;
            previous_changeset = node.changeset_id;
        }
        dense.set_denseinfo(dense_info);
        dense
    }

    fn encode_tags(&mut self, tags: Vec<Tag>) -> (Vec<u32>, Vec<u32>) {
        let mut keys: Vec<u32> = Vec::new();
        let mut vals: Vec<u32> = Vec::new();
        for tag in tags {
            keys.push(self.string_table.add(tag.key) as u32);
            vals.push(self.string_table.add(tag.value) as u32);
        }
        (keys, vals)
    }

    fn encode_nodes(&mut self, nodes: Vec<Node>) -> Vec<osmformat::Node> {
        nodes
            .into_iter()
            .map(|node| -> osmformat::Node {
                let mut osm_node = osmformat::Node::new();
                osm_node.set_id(node.id);
                osm_node.set_lat(self.codec.encode_latitude(node.latitude));
                osm_node.set_lon(self.codec.encode_longitude(node.longitude));

                let (keys, vals) = self.encode_tags(node.tags);
                osm_node.set_keys(keys);
                osm_node.set_vals(vals);

                let mut info = osmformat::Info::new();
                info.set_changeset(node.changeset_id);
                info.set_version(node.version);
                info.set_visible(node.visible);
                if let Some(timestamp) = node.timestamp {
                    info.set_timestamp(self.codec.encode_timestamp(timestamp));
                }
                if let Some(user) = node.user {
                    info.set_uid(user.id);
                    let sid = self.string_table.add(user.name);
                    info.set_user_sid(sid as u32);
                } else {
                    // No user: omit uid/user_sid entirely so readers map the element to
                    // "no user" (osmosis convention) instead of a phantom uid-0 user.
                }
                // Without this the Info is dropped and sparse nodes lose ALL metadata
                // (version, timestamp, changeset, user, visible) on the read side.
                osm_node.set_info(info);

                osm_node
            })
            .collect()
    }

    fn add_nodes(&mut self, nodes: Vec<Node>, use_dense: bool) {
        let mut group = osmformat::PrimitiveGroup::new();
        if use_dense {
            let dense = self.encode_dense_nodes(nodes);
            group.set_dense(dense);
        } else {
            let encoded_nodes = self.encode_nodes(nodes);
            group.set_nodes(RepeatedField::from_vec(encoded_nodes))
        }
        self.block.primitivegroup.push(group);
    }

    fn add_ways(&mut self, ways: Vec<Way>) {
        let encoded_ways: Vec<osmformat::Way> = ways
            .into_iter()
            .map(|way| {
                let mut osm_way = osmformat::Way::new();
                osm_way.set_id(way.id);

                let mut prev_ref_id = 0;
                osm_way.set_refs(
                    way.way_nodes
                        .into_iter()
                        .map(|way_node| {
                            let difference = way_node.id - prev_ref_id;
                            prev_ref_id = way_node.id;
                            difference
                        })
                        .collect(),
                );

                let (keys, vals) = self.encode_tags(way.tags);
                osm_way.set_keys(keys);
                osm_way.set_vals(vals);

                let mut info = osmformat::Info::new();
                info.set_changeset(way.changeset_id);
                info.set_version(way.version);
                info.set_visible(way.visible);
                if let Some(timestamp) = way.timestamp {
                    info.set_timestamp(self.codec.encode_timestamp(timestamp));
                } else {
                    info.set_timestamp(0);
                }
                if let Some(user) = way.user {
                    info.set_uid(user.id);
                    let sid = self.string_table.add(user.name);
                    info.set_user_sid(sid as u32);
                } else {
                    // No user: omit uid/user_sid entirely so readers map the element to
                    // "no user" (osmosis convention) instead of a phantom uid-0 user.
                }
                osm_way.set_info(info);

                osm_way
            })
            .collect();

        let mut group = osmformat::PrimitiveGroup::new();
        group.set_ways(RepeatedField::from_vec(encoded_ways));
        self.block.primitivegroup.push(group);
    }

    fn add_relations(&mut self, relations: Vec<Relation>) {
        let encoded_relations: Vec<osmformat::Relation> = relations
            .into_iter()
            .map(|relation| {
                let mut osm_relation = osmformat::Relation::new();
                osm_relation.set_id(relation.id);

                let mut prev_member_id = 0i64;
                for member in relation.members {
                    osm_relation.memids.push(member.member_id - prev_member_id);
                    prev_member_id = member.member_id;

                    osm_relation
                        .roles_sid
                        .push(self.string_table.add(member.role));
                    let osm_member_type = match member.member_type {
                        ElementType::Node => osmformat::Relation_MemberType::NODE,
                        ElementType::Way => osmformat::Relation_MemberType::WAY,
                        ElementType::Relation => osmformat::Relation_MemberType::RELATION,
                    };
                    osm_relation.types.push(osm_member_type);
                }

                let (keys, vals) = self.encode_tags(relation.tags);
                osm_relation.set_keys(keys);
                osm_relation.set_vals(vals);

                let mut info = osmformat::Info::new();
                info.set_changeset(relation.changeset_id);
                info.set_version(relation.version);
                info.set_visible(relation.visible);
                if let Some(timestamp) = relation.timestamp {
                    info.set_timestamp(self.codec.encode_timestamp(timestamp));
                } else {
                    info.set_timestamp(0);
                }
                if let Some(user) = relation.user {
                    info.set_uid(user.id);
                    let sid = self.string_table.add(user.name);
                    info.set_user_sid(sid as u32);
                } else {
                    // No user: omit uid/user_sid entirely so readers map the element to
                    // "no user" (osmosis convention) instead of a phantom uid-0 user.
                }
                osm_relation.set_info(info);

                osm_relation
            })
            .collect();

        let mut group = osmformat::PrimitiveGroup::new();
        group.set_relations(RepeatedField::from_vec(encoded_relations));
        self.block.primitivegroup.push(group);
    }

    /// Builds a `PrimitiveBlock` from the given elements.
    ///
    /// Ordering is the caller's responsibility: the PBF format does not require sorted ids
    /// for correct encoding (dense deltas handle any order), so no order validation is done
    /// here. Note that `IndexedReader` does require sorted input and rejects unordered files
    /// when building its index.
    pub fn build(mut self, elements: Vec<Element>, use_dense: bool) -> osmformat::PrimitiveBlock {
        let mut nodes = Vec::new();
        let mut ways = Vec::new();
        let mut relations = Vec::new();
        for element in elements {
            match element {
                Element::Node(node) => nodes.push(node),
                Element::Way(way) => ways.push(way),
                Element::Relation(relation) => relations.push(relation),
            }
        }
        // Dense encoding cannot represent a tag with an empty key/value: index 0 is the
        // reserved node terminator, so `add("")` resolves to 0 and the decoder would treat
        // the tag as the end of the node. Fall the whole block back to sparse nodes, where
        // index 0 is a legitimate string-table reference.
        //
        // Similarly, DenseInfo's timestamp column is a parallel array covering every node
        // or none, so a block mixing timestamped and timestamp-less nodes would silently
        // drop the timestamps of the former. Fall back to sparse encoding, where each
        // node's Info carries its own timestamp.
        let has_mixed_timestamps = {
            let has_timestamp = nodes.iter().any(|node| node.timestamp.is_some());
            let missing_timestamp = nodes.iter().any(|node| node.timestamp.is_none());
            has_timestamp && missing_timestamp
        };
        let use_dense = use_dense
            && !nodes.iter().any(|node| {
                node.tags
                    .iter()
                    .any(|tag| tag.key.is_empty() || tag.value.is_empty())
            })
            && !has_mixed_timestamps;
        if !nodes.is_empty() {
            self.add_nodes(nodes, use_dense);
        }
        if !ways.is_empty() {
            self.add_ways(ways);
        }
        if !relations.is_empty() {
            self.add_relations(relations);
        }

        self.block
            .set_stringtable(self.string_table.into_string_table());
        self.block
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    fn ts() -> DateTime<Utc> {
        let naive =
            chrono::NaiveDateTime::parse_from_str("2023-05-01 12:30:00", "%Y-%m-%d %H:%M:%S")
                .unwrap();
        DateTime::from_naive_utc_and_offset(naive, Utc)
    }

    fn node(id: i64) -> Node {
        Node {
            id,
            version: 1,
            timestamp: None,
            user: None,
            changeset_id: 1,
            latitude: 100 * id,
            longitude: 200 * id,
            visible: true,
            tags: Vec::new(),
        }
    }

    #[test]
    fn test_build() {
        let builder = PrimitiveBuilder::new();
        println!(
            "{}, {}",
            builder.block.get_granularity(),
            builder.block.get_date_granularity()
        );
    }

    #[test]
    fn mixed_timestamp_presence_falls_back_to_sparse() {
        // DenseInfo's timestamp column is all-or-nothing: a block mixing timestamped and
        // timestamp-less nodes must not be dense, otherwise the former would lose their
        // timestamps on roundtrip.
        let mut with_ts = node(1);
        with_ts.timestamp = Some(ts());
        let block = PrimitiveBuilder::new()
            .build(vec![Element::Node(with_ts), Element::Node(node(2))], true);
        let group = block.get_primitivegroup().first().unwrap();
        assert!(
            !group.has_dense(),
            "mixed timestamp presence must fall back to sparse encoding"
        );
        assert_eq!(group.get_nodes().len(), 2);
    }

    #[test]
    fn uniform_timestamps_stay_dense() {
        let mut first = node(1);
        first.timestamp = Some(ts());
        let mut second = node(2);
        second.timestamp = Some(ts());
        let block =
            PrimitiveBuilder::new().build(vec![Element::Node(first), Element::Node(second)], true);
        let group = block.get_primitivegroup().first().unwrap();
        assert!(group.has_dense(), "uniform timestamps must stay dense");
        assert_eq!(group.get_dense().get_id().len(), 2);
        assert_eq!(
            group.get_dense().get_denseinfo().get_timestamp().len(),
            2,
            "the dense timestamp column must cover every node"
        );
    }

    #[test]
    fn uniform_missing_timestamps_stay_dense() {
        let block = PrimitiveBuilder::new()
            .build(vec![Element::Node(node(1)), Element::Node(node(2))], true);
        let group = block.get_primitivegroup().first().unwrap();
        assert!(
            group.has_dense(),
            "uniformly timestamp-less must stay dense"
        );
        assert!(
            group.get_dense().get_denseinfo().get_timestamp().is_empty(),
            "the dense timestamp column must be omitted when every node lacks one"
        );
    }
}
