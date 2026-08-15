# pbf-craft-cli

`pbf-craft-cli` is a Rust-powered utility for inspecting, exporting, and comparing
OpenStreetMap `.osm.pbf` data. It pairs with the `pbf-craft` library and ships the same
decoding performance in a convenient command line. The binary is named `pbf-craft`.

## Installation

You need the Rust toolchain (`cargo`, Rust 1.70+ recommended). From the repository root:

```bash
cargo install pbf-craft-cli
```

Most commands operate on local `.osm.pbf` extracts. The `export` subcommand additionally
requires access to a Postgres database populated with OSM tables (`current_nodes`,
`current_ways`, etc.).

## Command Overview

- `get` – Fetch a specific node, way, or relation plus its referenced dependencies using the
  indexed reader cache.

  ```bash
  pbf-craft get --eltype way --elid 1055523837 --file resources/andorra-latest.osm.pbf
  ```

- `search` – Locate elements by id, tag filters, or node pair membership. `--tagkey` /
  `--tagvalue` are **substring** matches against tag keys/values. Use `--exact=false` to
  widen the search to related entities (elements referencing the target).

  ```bash
  pbf-craft search --tagkey highway --tagvalue residential --file data/map.osm.pbf
  pbf-craft search --eltype node --elid 123 --file data/map.osm.pbf --exact=false
  pbf-craft search --pair 123 456 --file data/map.osm.pbf
  ```

- `export` – Stream OSM objects from Postgres into a new PBF file. The password is read from
  `--password` or, when omitted, from the `PGPASSWORD` environment variable.

  ```bash
  pbf-craft export \
    --host localhost --port 5432 --user osm --dbname osm --output exports/latest.osm.pbf
  ```

- `boundary` – Compute the convex hull boundary of all points in a PBF extract and emit
  GeoJSON.

  ```bash
  pbf-craft boundary --file data/map.osm.pbf > boundary.geojson
  ```

- `diff` – Compare two PBF files and write a CSV of added, modified, and deleted elements
  (defaults to `./diff.csv`). Both files must follow the conventional (type, id) ordering
  (all nodes by id, then all ways, then all relations); unordered input is rejected with an
  error. A live progress line (percentage of each file consumed) is shown on stderr.

  ```bash
  pbf-craft diff --source old.osm.pbf --target new.osm.pbf --output reports/diff.csv
  ```

Run `pbf-craft <command> --help` for the complete flag reference of each subcommand,
including cache controls (`--cache-size`). All commands exit non-zero on fatal errors,
making them script-friendly.
