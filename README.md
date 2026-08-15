# pbf-craft

A pure-Rust workspace for reading and writing OpenStreetMap **PBF** (Protocolbuffer
Binary Format) files.

## Crates

| Crate | Description |
|---|---|
| [pbf-craft](pbf-craft/) | Library: OSM element models, streaming and indexed readers, and a PBF writer |
| [pbf-craft-cli](pbf-craft-cli/) | Command-line utility: `get`, `search`, `export`, `diff`, `boundary` |

## Features

- **Pure Rust**, no C dependencies.
- **Multiple readers** for different scenarios: sequential streaming ([`PbfReader`]),
  iterator-based with progress reporting ([`IterableReader`] + [`ReaderProgress`]), and
  random access by element id backed by a `.pif` index ([`IndexedReader`]), with an
  in-memory blob cache and dependency resolution.
- **Parallel filtering** ([`PbfReader::par_find`]).
- **Dense and sparse** node encoding; reads `raw`, `zlib`, `lz4` and `zstd` blobs; writes
  `zlib`-compressed blobs.
- **Result-based error handling**: malformed or truncated input surfaces as errors, not
  panics.
- **CLI**: fetch elements with dependencies, search by id/tag/node-pair, export from
  Postgres, diff two extracts, and compute an extract's boundary.

## Quick start (library)

Read a PBF file sequentially:

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

Find an element by id using the index:

```rust
use pbf_craft::models::ElementType;
use pbf_craft::readers::IndexedReader;

let mut indexed_reader = IndexedReader::from_path("resources/andorra-latest.osm.pbf").unwrap();
let node = indexed_reader.find(&ElementType::Node, 4254529698).unwrap();
```

Write a PBF file:

```rust
use pbf_craft::models::{Element, Node};
use pbf_craft::writers::PbfWriter;

let mut writer = PbfWriter::from_path(std::env::temp_dir().join("output.osm.pbf"), true).unwrap();
writer.write(Element::Node(Node::default())).unwrap();
writer.finish().unwrap();
```

## Quick start (CLI)

```bash
cargo run -p pbf-craft-cli -- get --eltype way --elid 1055523837 --file pbf-craft/resources/andorra-latest.osm.pbf
cargo run -p pbf-craft-cli -- search --tagkey highway --file pbf-craft/resources/andorra-latest.osm.pbf
cargo run -p pbf-craft-cli -- boundary --file pbf-craft/resources/andorra-latest.osm.pbf
```

See [pbf-craft-cli](pbf-craft-cli/) for the full command reference.

## Data format notes

- **Coordinates**: `Node`/`WayNode`/`Bound` coordinates are i64 **nanodegrees** (the raw
  PBF unit; divide by 1e9 for degrees).
- **Ordering**: the PBF format does not require sorted elements, but the conventional
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

## Development

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

## License

MIT
