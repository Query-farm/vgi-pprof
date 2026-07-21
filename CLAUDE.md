# CLAUDE.md — vgi-pprof

Contributor/agent notes. User-facing docs live in `README.md`; this is the
"how it's built and where the sharp edges are" companion.

## What this is

A [VGI](https://query.farm) worker (Rust, compiled binary) that decodes **pprof**
profiles (the gzip-wrapped `profile.proto` emitted by Go, gperftools, Parca,
Pyroscope, py-spy, …) into SQL rows over Arrow IPC. Built on the `vgi` crate
(crates.io), modeled on `vgi-fixedformat` / `vgi-units`. Catalog name `pprof`
(single `main` schema). It exists so SRE/perf teams can **bulk-diff profiles in
SQL** — `go tool pprof` is interactive and single-file.

Distinct format/library from `vgi-perf` (Linux `perf.data`) — do **not** merge
them; they ship in one observability/CI-regression bundle but decode different
things.

## Layout

```
Cargo.toml                          workspace; pins vgi = "0.18.0" (→ vgi-rpc 0.11, arrow 59), prost 0.14
crates/pprof-core/                  PURE decode/flatten engine (no Arrow/VGI)
  proto/profile.proto               VENDORED Google pprof schema (Apache-2.0)
  build.rs                          prost-build codegen; protoc via protoc-bin-vendored (hermetic)
  src/lib.rs                        gunzip + prost decode + flatten into row structs + unit tests
  tests/golden.rs                   golden fixtures (one per producer) + proptest no-panic fuzzing
crates/pprof-worker/
  src/main.rs                       Worker::new(); registers tables; catalog metadata (incl. implementation_version) + the VALUES-backed sample_type_guide view
  src/source.rs                     resolve overloaded `src` (path/glob/list/BLOB) + per-file error capture
  src/arrow_build.rs                row structs -> Arrow columns (LIST/STRUCT/MAP type defs shared with on_bind)
  src/table/{stacks,samples,functions,locations,mappings,meta,mod}.rs   thin table-fn adapters
  src/meta.rs                       vgi-lint metadata tag helpers (shared)
data/generate_fixtures.go           builds the golden .pb.gz fixtures with google/pprof
data/*.pb.gz                        committed fixtures (+ empty.pb.gz / bad.pb.gz for error tests)
test/sql/*.test                     haybarn-unittest sqllogictest — authoritative E2E
ci/                                 run-integration.sh (transport matrix) + check-version.sh + preprocess-require.awk
run_tests.sh                        local E2E convenience wrapper
```

Pattern: keep decode/flatten in `pprof-core` (pure, golden + proptest tested),
keep Arrow marshalling in `pprof-worker` (thin). A nested column's Arrow type is
defined **once** in `arrow_build.rs` and reused by both `output_schema()` (what
`on_bind` returns) and the column builder, so the declared schema and the built
`RecordBatch` can't drift.

## The decode model

`profile.proto` is a string-table-indexed graph: samples reference location ids
(leaf first), locations belong to a mapping and carry inlined `Line`s (innermost
first), each line names a function, and all the human strings live in
`string_table` (index 0 is always `""`). `pprof-core` resolves those indices once
and pre-joins the graph:

- **`stacks`** (the headline) — one row per sample with `value` (LIST(BIGINT)
  aligned to `meta.sample_types`), `labels` (MAP), and `frame` (LIST(STRUCT),
  leaf first, inlined frames expanded). An unsymbolized location yields one
  address-only frame (function/filename NULL, address kept).
- **`samples`/`functions`/`locations`/`mappings`** — the raw graph with the
  profile's **original ids passed through verbatim** so they join (and so
  `mappings.build_id` flows to `vgi-symbols`).
- **`meta`** — one row of sample types / period / duration.

## Sharp edges

1. **Per-file error capture, not abort.** Every table appends `file` + `error`
   columns. A missing/zero-byte/malformed profile becomes a single error row
   (data columns NULL); a glob mixing good and bad files still returns the good
   rows. Always filter `WHERE error IS NULL` in aggregations.
2. **`src` overloads on type.** VARCHAR → path (may glob); LIST(VARCHAR) → many
   paths; BLOB → inline bytes (`file` is NULL). See `source.rs`. `read_text` is a
   DuckDB *table* function and can't be nested as a scalar arg — for inline bytes
   use a BLOB literal (`from_base64('…')`) or `read_blob(...)` in a FROM clause.
3. **gzip is sniffed, not assumed.** On-disk pprof is gzip-wrapped, but some
   producers/tests emit raw protobuf; `Decoded::from_bytes` checks the `1f 8b`
   magic and inflates only when present.
4. **MAP labels need unique keys.** DuckDB's MAP rejects duplicate keys, so
   `core` de-duplicates label keys (first wins). pprof itself discourages
   multi-value label keys. Numeric labels render as the number (+ unit suffix).
5. **value indexing is 1-based and type-ordered.** For a Go CPU profile
   `value[1]` is `samples/count` and `value[2]` is `cpu/nanoseconds` — read
   `pprof.meta(src).sample_types` to know which slot is which.
6. **`haybarn-unittest` skips `require vgi`** — `.test` files use explicit
   `statement ok` + `LOAD vgi;`. After `SET search_path = 'pprof.main'` the
   `pprof` catalog is the *default* database, so do **not** `DETACH pprof` at
   end-of-file (it errors). Fixtures are referenced by absolute path via
   `${VGI_PPROF_DATA}` because the runner cd's into a staging dir.
7. **prost-build needs `protoc`** — supplied hermetically by
   `protoc-bin-vendored` in `build.rs`, so neither CI nor a dev box needs a
   system `protoc`. An explicit `PROTOC` env still wins if set.

## Gates (all green)

`cargo build --release` · `cargo clippy --all-targets -- -D warnings` ·
`cargo fmt --check` · `cargo test` · `cargo doc` (RUSTDOCFLAGS=`-D warnings`) ·
`vgi-lint … --fail-on info` (100/100) · haybarn SQLLogic E2E on
subprocess/http/unix.

## License

MIT (fleet convention). The vendored `profile.proto` is Apache-2.0 (Google).
Copyright 2026 Query Farm LLC — https://query.farm
