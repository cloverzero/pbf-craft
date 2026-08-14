# pbf-craft

A Rust library and command-line tool for reading and writing OpenStreetMap PBF file format.

It contains a variety of PBF readers for a variety of scenarios. For example, a PBF
reader with an index can locate and read elements more efficiently. It also provides
a PBF writer that can write PBF data to a file.

- Written in pure Rust
- Provides an indexing feature to the PBF to greatly improve read performance.

## Example

Reading a PBF file:

```rust
use pbf_craft::readers::PbfReader;

let mut reader = PbfReader::from_path("resources/andorra-latest.osm.pbf").unwrap();
reader.read(|header, element| {
    if let Some(header_reader) = header {
        // Process header
    }
    if let Some(element) = element {
        // Process element
    }
}).unwrap();
```

Finding an element using the index feature. `IndexedReader` creates an index file for the PBF file, which allows you to quickly locate and retrieve an element when looking for it using its ID. `IndexedReader` has an cache option, with which you can fetch a element with its dependencies more efficiently.

```rust
use pbf_craft::models::ElementType;
use pbf_craft::readers::IndexedReader;

let mut indexed_reader = IndexedReader::from_path_with_cache("resources/andorra-latest.osm.pbf", 1000).unwrap();
let node = indexed_reader.find(&ElementType::Node, 12345678).unwrap();
let element_list = indexed_reader.get_with_deps(&ElementType::Way, 1055523837).unwrap();
```

Writing a PBF file:

```rust
use pbf_craft::models::{Element, Node};
use pbf_craft::writers::PbfWriter;

let mut writer = PbfWriter::from_path(std::env::temp_dir().join("output.osm.pbf"), true).unwrap();
writer.write(Element::Node(Node::default())).unwrap();
writer.finish().unwrap();
```

## Data format notes

- **Coordinates**: `Node.latitude` / `Node.longitude` are i64 **nanodegrees** (the raw PBF
  unit; divide by 1e9 for degrees). `Bound` fields are nanodegrees as well.
- **Sorting**: the PBF format does not require sorted elements, but the conventional file
  layout (all nodes by id, then all ways by id, then all relations by id) is assumed by
  `IndexedReader` and most other tools. `PbfWriter` stores elements in the order written —
  the caller is responsible for the order; `IndexedReader` rejects unordered files with an
  error when building its index.
- **Compression**: reading supports `raw`, `zlib`, `lz4` and `zstd` blobs; writing produces
  `zlib`-compressed blobs.
- **visible flag**: elements default to `visible = true` (per spec, the flag is assumed true
  when absent). Elements explicitly marked `visible = false` are written with the required
  `HistoricalInformation` feature declared in the header.
- **Indexing**: `IndexedReader` builds a `.pif` index validated against the PBF file's size
  and mtime; unsorted files are rejected with an error rather than silently returning wrong
  lookup results.
