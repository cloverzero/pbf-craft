# pbf-craft

A pure-Rust library for reading and writing OpenStreetMap **PBF** (Protocolbuffer Binary
Format) files.

## Features

- **Multiple readers** for different scenarios:
  - [`PbfReader`](https://docs.rs/pbf-craft/latest/pbf_craft/readers/struct.PbfReader.html) —
    sequential streaming reader, with parallel filtering
    ([`par_find`](https://docs.rs/pbf-craft/latest/pbf_craft/readers/struct.PbfReader.html#method.par_find))
    and byte progress reporting.
  - [`IterableReader`](https://docs.rs/pbf-craft/latest/pbf_craft/readers/struct.IterableReader.html) —
    iterator-based reader with [`ReaderProgress`](https://docs.rs/pbf-craft/latest/pbf_craft/readers/struct.ReaderProgress.html).
  - [`IndexedReader`](https://docs.rs/pbf-craft/latest/pbf_craft/readers/struct.IndexedReader.html) —
    random access by element id, backed by a `.pif` index file, with an in-memory blob cache
    and dependency resolution.
- **Dense and sparse** node encoding; reads `raw`, `zlib`, `lz4` and `zstd` blobs; writes
  `zlib`-compressed blobs.
- **Result-based error handling**: malformed or truncated input surfaces as errors, not
  panics.
- Pure Rust, no C dependencies.

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
pbf-craft = "1"
```

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

Finding an element using the index feature. `IndexedReader` creates an index file for the
PBF file, which allows you to quickly locate and retrieve an element when looking for it
using its ID. `IndexedReader` has a cache option, with which you can fetch an element with
its dependencies more efficiently.

```rust
use pbf_craft::models::ElementType;
use pbf_craft::readers::IndexedReader;

let mut indexed_reader = IndexedReader::from_path_with_cache("resources/andorra-latest.osm.pbf", 1000).unwrap();
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
- **Error handling**: all fallible operations return `anyhow::Result`; malformed input
  surfaces as errors rather than panics.

## MSRV

Current stable Rust (1.70+). This crate is tested against the latest stable toolchain.

## License

MIT
