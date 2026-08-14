//! Regression tests for confirmed data-loss and robustness bugs (see FIX_PLAN.md).
//!
//! Status after Phase 3 (all data-loss and robustness fixes landed): every test here is
//! expected to pass.
use std::path::PathBuf;

use pbf_craft::models::{
    Element, ElementType, Node, OsmUser, Relation, RelationMember, Tag, Way, WayNode,
};
use pbf_craft::readers::{CachedReader, IndexedReader, PbfReader};
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

// Minimal hand-rolled protobuf encoding helpers so tests can craft PBF files the writer
// refuses to produce (e.g. unsorted element ids).

fn varint(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// A sparse `Node` message: id (field 1, sint64), lat (field 8), lon (field 9); no tags/info.
fn node_bytes(id: i64) -> Vec<u8> {
    let mut b = Vec::new();
    b.push(0x08);
    varint(zigzag(id), &mut b);
    b.push(0x40);
    varint(zigzag(id * 100), &mut b);
    b.push(0x48);
    varint(zigzag(id * 200), &mut b);
    b
}

fn string_table_bytes() -> Vec<u8> {
    // StringTable { s: [""] } — one empty entry at index 0.
    vec![0x0a, 0x00]
}

fn primitive_block_bytes(nodes: &[i64]) -> Vec<u8> {
    let st = string_table_bytes();
    let mut group = Vec::new();
    for id in nodes {
        let n = node_bytes(*id);
        group.push(0x0a); // PrimitiveGroup.nodes (field 1, message)
        varint(n.len() as u64, &mut group);
        group.extend_from_slice(&n);
    }
    let mut b = Vec::new();
    b.push(0x0a); // PrimitiveBlock.stringtable (field 1, message)
    varint(st.len() as u64, &mut b);
    b.extend_from_slice(&st);
    b.push(0x12); // PrimitiveBlock.primitivegroup (field 2, message)
    varint(group.len() as u64, &mut b);
    b.extend_from_slice(&group);
    b
}

fn frame(blob_type: &str, raw: &[u8]) -> Vec<u8> {
    // Blob { raw: raw } (field 1, bytes)
    let mut blob = Vec::new();
    blob.push(0x0a);
    varint(raw.len() as u64, &mut blob);
    blob.extend_from_slice(raw);
    // BlobHeader { type, datasize }
    let mut header = Vec::new();
    header.push(0x0a);
    varint(blob_type.len() as u64, &mut header);
    header.extend_from_slice(blob_type.as_bytes());
    header.push(0x18);
    varint(blob.len() as u64, &mut header);
    // u32 BE header length + header + blob
    let mut out = Vec::new();
    out.extend_from_slice(&(header.len() as u32).to_be_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(&blob);
    out
}

/// A valid PBF (header + one data blob) with the given node ids per blob.
fn crafted_pbf(node_blobs: &[&[i64]]) -> Vec<u8> {
    let mut header_raw = Vec::new();
    header_raw.push(0x22); // HeaderBlock.required_features (field 4, string)
    varint("OsmSchema-V0.6".len() as u64, &mut header_raw);
    header_raw.extend_from_slice(b"OsmSchema-V0.6");
    let mut out = frame("OSMHeader", &header_raw);
    for nodes in node_blobs {
        out.extend_from_slice(&frame("OSMData", &primitive_block_bytes(nodes)));
    }
    out
}

#[test]
fn unsorted_file_index_returns_error() {
    // The index maps "last element id per blob" -> blob offset and relies on the file being
    // sorted by id. A file with disorder (hand-crafted, since the writer now rejects it) must
    // fail loudly at index build time instead of silently missing elements later.
    let file = TempPbf::new("unsorted_index");
    // Blob 1: ids 1..3; blob 2: ids 2..4 — not monotonically increasing across blobs.
    std::fs::write(file.as_ref(), crafted_pbf(&[&[1, 2, 3], &[2, 3, 4]])).unwrap();

    let err = match IndexedReader::from_path(file.as_ref().to_str().unwrap()) {
        Ok(_) => panic!("unsorted file must be rejected by the index"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("not sorted"),
        "expected a 'not sorted' error, got: {}",
        err
    );
}

#[test]
fn crafted_sorted_file_is_indexable() {
    // Sanity check for the hand-rolled encoder: a sorted crafted file must index and find.
    let file = TempPbf::new("sorted_index");
    std::fs::write(file.as_ref(), crafted_pbf(&[&[1, 2, 3], &[4, 5, 6]])).unwrap();

    let mut indexed = IndexedReader::from_path(file.as_ref().to_str().unwrap()).unwrap();
    let node = indexed.find_node(5).unwrap();
    assert!(
        node.is_some(),
        "crafted sorted file must support indexed lookup"
    );
    assert_eq!(node.unwrap().id, 5);
}

#[test]
fn writer_drop_flushes_buffered_elements() {
    // A forgotten finish() must not silently produce an empty file: Drop flushes the buffer.
    let file = TempPbf::new("drop_flush");
    {
        let mut writer = PbfWriter::from_path(file.as_ref(), true).unwrap();
        writer.write(Element::Node(base_node(1))).unwrap();
        // no finish()
    }
    let mut reader = PbfReader::from_path(file.as_ref()).unwrap();
    let mut count = 0;
    reader
        .read(|_, el| {
            if el.is_some() {
                count += 1;
            }
        })
        .unwrap();
    assert_eq!(
        count, 1,
        "dropped writer must still flush buffered elements"
    );
}

#[test]
fn finish_without_elements_writes_valid_header_only() {
    let file = TempPbf::new("empty_finish");
    {
        let mut writer = PbfWriter::from_path(file.as_ref(), true).unwrap();
        writer.finish().unwrap();
    }
    let mut reader = PbfReader::from_path(file.as_ref()).unwrap();
    let mut count = 0;
    reader
        .read(|_, el| {
            if el.is_some() {
                count += 1;
            }
        })
        .unwrap();
    assert_eq!(count, 0, "empty finish() must not emit an empty data block");
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

#[test]
fn roundtrip_user_presence() {
    // N1: a userless element must read back as user=None (not a phantom uid-0 user), and a
    // real user must keep its id/name — in both dense and sparse encodings.
    let mut with_user = base_node(1);
    with_user.user = Some(OsmUser {
        id: 42,
        name: "mapper".to_string(),
    });
    let without_user = base_node(2);
    for use_dense in [true, false] {
        let out = write_and_read_nodes(
            &format!("user_presence_{}", use_dense),
            vec![with_user.clone(), without_user.clone()],
            use_dense,
        );
        assert_eq!(out.len(), 2);
        assert_eq!(
            out[0].user,
            Some(OsmUser {
                id: 42,
                name: "mapper".to_string()
            }),
            "user must survive roundtrip (dense={})",
            use_dense
        );
        assert!(
            out[1].user.is_none(),
            "userless element must read back as no user (dense={})",
            use_dense
        );
    }
}

#[test]
fn par_find_matches_sequential_read() {
    // Regression guard for the par_find pipeline: parallel filtering must return exactly the
    // same elements as a sequential read on the committed fixture.
    let file = "resources/andorra-latest.osm.pbf";
    let mut reader = PbfReader::from_path(file).unwrap();
    let mut seq_nodes = 0usize;
    let mut seq_ways = 0usize;
    reader
        .read(|_, el| match el {
            Some(Element::Node(n)) if n.id % 1000 == 0 => seq_nodes += 1,
            Some(Element::Way(w)) if w.tags.iter().any(|t| t.key == "highway") => seq_ways += 1,
            _ => {}
        })
        .unwrap();

    let reader = PbfReader::from_path(file).unwrap();
    let found = reader
        .par_find(None, |el| match el {
            Element::Node(n) => n.id % 1000 == 0,
            Element::Way(w) => w.tags.iter().any(|t| t.key == "highway"),
            _ => false,
        })
        .unwrap();
    assert_eq!(
        found
            .iter()
            .filter(|e| matches!(e, Element::Node(_)))
            .count(),
        seq_nodes,
        "par_find node count must match sequential read"
    );
    assert_eq!(
        found
            .iter()
            .filter(|e| matches!(e, Element::Way(_)))
            .count(),
        seq_ways,
        "par_find way count must match sequential read"
    );
}

#[test]
fn iterable_reader_reports_progress() {
    // >8000 elements force multiple blobs, so byte progress advances in steps and ends at 100%.
    let file = TempPbf::new("progress");
    let mut writer = PbfWriter::from_path(file.as_ref(), true).unwrap();
    let total = 16_010i64; // two full blocks + a partial one
    for id in 1..=total {
        writer.write(Element::Node(base_node(id))).unwrap();
    }
    writer.finish().unwrap();

    let mut reader = pbf_craft::readers::IterableReader::from_path(file.as_ref()).unwrap();
    let mut consumed = 0u64;
    let mut mid_percent = None;
    let mut prev = 0.0f64;
    while let Some(_element) = reader.next() {
        consumed += 1;
        let percent = reader
            .progress()
            .percent()
            .expect("from_path knows the total");
        assert!(
            percent >= prev,
            "percent must be monotonic ({} then {})",
            prev,
            percent
        );
        prev = percent;
        if consumed == 4001 {
            mid_percent = Some(percent);
        }
    }
    assert_eq!(consumed, total as u64);
    let mid = mid_percent.expect("mid-progress must be sampled");
    assert!(
        (0.0..100.0).contains(&mid),
        "mid-iteration progress must be strictly between 0% and 100%, got {}",
        mid
    );
    assert_eq!(
        reader.progress().percent(),
        Some(100.0),
        "EOF must report 100%"
    );
}

#[test]
fn reader_progress_unknown_total() {
    // A non-file stream has no known length: percent must be None.
    let reader = pbf_craft::readers::PbfReader::new(std::io::Cursor::new(Vec::<u8>::new()));
    let progress = reader.progress();
    assert_eq!(progress.total_bytes, None);
    assert_eq!(progress.percent(), None);
}

#[test]
fn cached_reader_inherits_progress_via_deref() {
    // CachedReader derefs to PbfReader, so progress() is available with zero extra code.
    let file = TempPbf::new("cached_progress");
    let mut writer = PbfWriter::from_path(file.as_ref(), true).unwrap();
    writer.write(Element::Node(base_node(1))).unwrap();
    writer.finish().unwrap();
    let expected_len = std::fs::metadata(file.as_ref()).unwrap().len();

    let reader = pbf_craft::readers::PbfReader::from_path(file.as_ref()).unwrap();
    let cached = CachedReader::new(reader, 10);
    assert_eq!(cached.progress().total_bytes, Some(expected_len));
}
