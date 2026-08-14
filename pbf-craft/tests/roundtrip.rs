//! Regression tests for confirmed data-loss and robustness bugs (see FIX_PLAN.md).
//!
//! Status matrix after Phase 1 (all data-loss bugs fixed):
//!
//! | test                                       | status | fix ref |
//! |--------------------------------------------|--------|---------|
//! | dense_roundtrip_preserves_visible_false    | PASS   | 1.2     |
//! | sparse_roundtrip_preserves_visible_false   | PASS   | 1.2     |
//! | roundtrip_preserves_timestamp_none         | PASS   | 1.3     |
//! | roundtrip_empty_tag_key                    | PASS   | 1.1     |
//! | roundtrip_tag_key_equals_first_string      | PASS   | 1.1     |
//! | relation_cycle_returns_error               | PASS   | 1.4     |
//! | roundtrip_mixed_types_and_multi_block      | PASS   | -       |
//! | truncated_file_returns_error               | PASS   | 2.4     |
//! | unsorted_file_index_silent_miss            | FAIL   | 3.2     |
//!
//! `unsorted_file_index_silent_miss` stays red until FIX_PLAN 3.2 (index must detect and
//! error on unsorted files instead of silently missing elements).
use std::path::PathBuf;

use pbf_craft::models::{
    Element, ElementType, Node, OsmUser, Relation, RelationMember, Tag, Way, WayNode,
};
use pbf_craft::readers::{IndexedReader, PbfReader};
use pbf_craft::writers::PbfWriter;

/// A unique temp file path per test; removed on drop.
struct TempPbf {
    path: PathBuf,
}

impl TempPbf {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "pbf_craft_roundtrip_{}_{}.pbf",
            std::process::id(),
            name
        ));
        Self { path }
    }
}

impl Drop for TempPbf {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl AsRef<std::path::Path> for TempPbf {
    fn as_ref(&self) -> &std::path::Path {
        &self.path
    }
}

fn base_node(id: i64) -> Node {
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

fn write_and_read_nodes(name: &str, nodes: Vec<Node>, use_dense: bool) -> Vec<Node> {
    // Each caller passes a unique `name` — tests run in parallel and must not share a temp file.
    let file = TempPbf::new(name);
    let mut writer = PbfWriter::from_path(file.as_ref(), use_dense).unwrap();
    for node in nodes {
        writer.write(Element::Node(node)).unwrap();
    }
    writer.finish().unwrap();

    let mut reader = PbfReader::from_path(file.as_ref()).unwrap();
    let mut out = Vec::new();
    reader
        .read(|_, element| {
            if let Some(Element::Node(node)) = element {
                out.push(node);
            }
        })
        .unwrap();
    out
}

#[test]
fn dense_roundtrip_preserves_visible_false() {
    let node = Node {
        visible: false, // deleted/historical object
        ..base_node(1)
    };
    let out = write_and_read_nodes("dense_visible_false", vec![node], true);
    assert_eq!(out.len(), 1);
    assert!(
        !out[0].visible,
        "visible=false must survive dense roundtrip"
    );
}

#[test]
fn sparse_roundtrip_preserves_visible_false() {
    let node = Node {
        visible: false,
        ..base_node(1)
    };
    let out = write_and_read_nodes("sparse_visible_false", vec![node], false);
    assert_eq!(out.len(), 1);
    assert!(
        !out[0].visible,
        "visible=false must survive sparse roundtrip"
    );
}

#[test]
fn roundtrip_preserves_timestamp_none() {
    // timestamp None must stay None — currently it comes back as Some(1970-01-01).
    let out = write_and_read_nodes("timestamp_none", vec![base_node(1)], true);
    assert_eq!(out.len(), 1);
    assert!(
        out[0].timestamp.is_none(),
        "timestamp None must stay None, got {:?}",
        out[0].timestamp
    );
}

#[test]
fn roundtrip_empty_tag_key() {
    // PBF reserves string-table index 0 as the dense terminator; the writer must not emit a
    // real tag with id 0, otherwise the tag is silently dropped / the key-value stream shifts.
    let mut node = base_node(1);
    node.tags = vec![Tag {
        key: String::new(),
        value: "foo".to_string(),
    }];
    let out = write_and_read_nodes("empty_tag_key", vec![node], true);
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].tags,
        vec![Tag {
            key: String::new(),
            value: "foo".to_string()
        }],
        "empty-key tag must survive dense roundtrip"
    );
}

#[test]
fn roundtrip_tag_key_equals_first_string() {
    // The first string added to the block's string table currently occupies index 0 (the
    // reserved delimiter). A tag key equal to the first node's username then collides and is
    // silently dropped.
    let mut n1 = base_node(1);
    n1.user = Some(OsmUser {
        id: 1,
        name: "highway".to_string(),
    });
    let mut n2 = base_node(2);
    n2.tags = vec![Tag {
        key: "highway".to_string(),
        value: "residential".to_string(),
    }];
    let out = write_and_read_nodes("tag_key_first_string", vec![n1, n2], true);
    assert_eq!(out.len(), 2);
    assert_eq!(
        out[1].tags,
        vec![Tag {
            key: "highway".to_string(),
            value: "residential".to_string()
        }],
        "tag key equal to the first string (index 0) must survive dense roundtrip"
    );
}

#[test]
fn roundtrip_mixed_types_and_multi_block() {
    // > MAX_BLOCK_ITEM_LENGTH elements force multiple blocks; nodes+ways+relations mixed.
    let file = TempPbf::new("mixed_multi_block");
    let mut writer = PbfWriter::from_path(file.as_ref(), true).unwrap();

    let node_count = 8100;
    for id in 1..=node_count {
        let node = Node {
            tags: vec![Tag {
                key: "k".to_string(),
                value: format!("v{}", id),
            }],
            ..base_node(id)
        };
        writer.write(Element::Node(node)).unwrap();
    }
    for id in 1..=100 {
        let way = Way {
            id,
            version: 1,
            tags: vec![Tag {
                key: "highway".to_string(),
                value: "residential".to_string(),
            }],
            way_nodes: vec![
                WayNode::new_without_coords(id * 10),
                WayNode::new_without_coords(id * 10 + 1),
            ],
            ..Default::default()
        };
        writer.write(Element::Way(way)).unwrap();
    }
    for id in 1..=50 {
        let relation = Relation {
            id,
            version: 1,
            members: vec![RelationMember {
                member_id: id * 10,
                member_type: ElementType::Node,
                role: "member".to_string(),
            }],
            ..Default::default()
        };
        writer.write(Element::Relation(relation)).unwrap();
    }
    writer.finish().unwrap();

    let mut reader = PbfReader::from_path(file.as_ref()).unwrap();
    let mut nodes = Vec::new();
    let mut ways = Vec::new();
    let mut relations = Vec::new();
    reader
        .read(|_, element| match element {
            Some(Element::Node(n)) => nodes.push(n),
            Some(Element::Way(w)) => ways.push(w),
            Some(Element::Relation(r)) => relations.push(r),
            None => {}
        })
        .unwrap();

    assert_eq!(nodes.len(), node_count as usize, "node count across blocks");
    assert_eq!(ways.len(), 100, "way count");
    assert_eq!(relations.len(), 50, "relation count");
    assert_eq!(nodes[0].id, 1);
    assert_eq!(nodes.last().unwrap().id, node_count);
    assert_eq!(nodes[0].tags[0].value, "v1");
    assert_eq!(nodes[8000].tags[0].value, "v8001", "second block node");
    assert_eq!(ways[0].way_nodes.len(), 2);
    assert_eq!(relations[0].members.len(), 1);
}

#[test]
fn unsorted_file_index_silent_miss() {
    // The index maps "last element id per blob" -> blob offset and relies on the file being
    // sorted by id. An unsorted file currently makes find_node() silently return None for an
    // element that exists in the file. Desired (FIX_PLAN 3.2): detect and error instead.
    let file = TempPbf::new("unsorted_index");
    let mut writer = PbfWriter::from_path(file.as_ref(), true).unwrap();
    // Block 1: ids 1..=8000 (flushed when the cache reaches 8000).
    for id in 1..=8000 {
        writer.write(Element::Node(base_node(id))).unwrap();
    }
    // Block 2 (partial, flushed by finish()): overlapping lower ids 4801..=5600.
    for id in 4801..=5600 {
        writer.write(Element::Node(base_node(id))).unwrap();
    }
    writer.finish().unwrap();

    // Prove the element exists via a sequential read.
    let mut reader = PbfReader::from_path(file.as_ref()).unwrap();
    let mut found = false;
    reader
        .read(|_, element| {
            if let Some(Element::Node(n)) = element {
                if n.id == 3000 {
                    found = true;
                }
            }
        })
        .unwrap();
    assert!(found, "node 3000 must exist in the file");

    // Indexed lookup currently resolves 3000 to block 2's offset and misses it.
    let mut indexed = IndexedReader::from_path(file.as_ref().to_str().unwrap()).unwrap();
    let node = indexed.find_node(3000).unwrap();
    assert!(
        node.is_some(),
        "indexed lookup of an existing node must not silently miss on unsorted files"
    );
}

#[test]
fn truncated_file_returns_error() {
    // Cutting bytes off the end must surface as an error, not as silently truncated data.
    let file = TempPbf::new("truncated");
    let mut writer = PbfWriter::from_path(file.as_ref(), true).unwrap();
    for id in 1..=100 {
        writer.write(Element::Node(base_node(id))).unwrap();
    }
    writer.finish().unwrap();

    let data = std::fs::read(file.as_ref()).unwrap();
    let truncated = data[..data.len() - 10].to_vec();
    let trunc_path = std::env::temp_dir().join(format!(
        "pbf_craft_roundtrip_{}_truncated_cut.pbf",
        std::process::id()
    ));
    std::fs::write(&trunc_path, truncated).unwrap();

    let mut reader = PbfReader::from_path(&trunc_path).unwrap();
    let result = reader.read(|_, _| {});
    std::fs::remove_file(&trunc_path).unwrap();
    assert!(
        result.is_err(),
        "truncated file must produce an error, got Ok"
    );
}

#[test]
fn relation_cycle_returns_error() {
    let file = TempPbf::new("relation_cycle");
    let mut writer = PbfWriter::from_path(file.as_ref(), true).unwrap();
    let r1 = Relation {
        id: 1,
        members: vec![RelationMember {
            member_id: 2,
            member_type: ElementType::Relation,
            role: "cycle".to_string(),
        }],
        ..Default::default()
    };
    let r2 = Relation {
        id: 2,
        members: vec![RelationMember {
            member_id: 1,
            member_type: ElementType::Relation,
            role: "cycle".to_string(),
        }],
        ..Default::default()
    };
    writer.write(Element::Relation(r1)).unwrap();
    writer.write(Element::Relation(r2)).unwrap();
    writer.finish().unwrap();

    let mut indexed = IndexedReader::from_path(file.as_ref().to_str().unwrap()).unwrap();
    let result = indexed.get_with_deps(&ElementType::Relation, 1);
    assert!(
        result.is_err(),
        "circular relation dependency must return an error, not crash"
    );
}

#[test]
fn default_elements_are_visible() {
    // Rust's derived Default would give `bool` = false, silently marking every fresh element
    // as deleted on write. The PBF spec says visible MUST be assumed true when absent, so the
    // model defaults it to true.
    assert!(Node::default().visible, "Node::default() must be visible");
    assert!(Way::default().visible, "Way::default() must be visible");
    assert!(
        Relation::default().visible,
        "Relation::default() must be visible"
    );
    assert!(
        pbf_craft::models::ElementBase::default().visible,
        "ElementBase::default() must be visible"
    );
}

#[test]
fn roundtrip_default_elements_stay_visible() {
    // The README/doc example writes `Element::Node(Node::default())`; the output must contain
    // visible (i.e. current, non-deleted) elements, in both dense and sparse encodings.
    for use_dense in [true, false] {
        let out = write_and_read_nodes(
            &format!("default_visible_{}", use_dense),
            vec![Node::default()],
            use_dense,
        );
        assert_eq!(out.len(), 1);
        assert!(
            out[0].visible,
            "default-constructed node must roundtrip as visible (dense={})",
            use_dense
        );
    }
}

#[test]
fn writer_declares_historical_information_for_invisible() {
    // Per spec, writing visible=false requires the HistoricalInformation feature, and the
    // reader must not reject such files (it decodes the visible flag).
    let file = TempPbf::new("historical");
    let mut writer = PbfWriter::from_path(file.as_ref(), true).unwrap();
    let node = Node {
        visible: false,
        ..base_node(1)
    };
    writer.write(Element::Node(node)).unwrap();
    writer.finish().unwrap();

    let mut reader = PbfReader::from_path(file.as_ref()).unwrap();
    let mut features = Vec::new();
    let mut read_visible = None;
    reader
        .read(|header, element| {
            if let Some(header_reader) = header {
                features = header_reader.required_features();
            }
            if let Some(Element::Node(n)) = element {
                read_visible = Some(n.visible);
            }
        })
        .unwrap();

    assert!(
        features.iter().any(|f| f == "HistoricalInformation"),
        "header must declare HistoricalInformation when visible=false is written, got {:?}",
        features
    );
    assert_eq!(
        read_visible,
        Some(false),
        "visible=false must survive roundtrip"
    );
}
