# Changelog

All notable changes to `vgi-pprof` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/), and the project adheres to
[Semantic Versioning](https://semver.org/).

## 0.5.1 (2026-11-25)

- CI: grant `checks: write` to the cargo-audit job (and its release/image callers) so rustsec/audit-check can post its check-run; the new unmaintained-crate advisories (chrono RUSTSEC-2026-0543, humantime RUSTSEC-2026-0542) made it report and fail on the read-only token, blocking the 0.5.0 release.

## 0.5.0 (2026-11-25)

- VGI protocol 3.4: `vgi` 0.53.2, `vgi-rpc` 0.33.1 (Orchard hosted mode; `blob-vendor` on the native build). Requires the 3.4 community `vgi` extension (haybarn 2.0.1).
- SQL tests run on haybarn-unittest 2.0.1rc3. Dockerfile HEALTHCHECK probes `HEAD /health` (hosted mode answers anonymous GET with 401).

## 0.4.3 (2026-11-16)

- Bump the vgi Rust SDK to 0.51.1.
- CI: a release reuses the passing CI run of its commit; container images build on release tags only.

## [0.4.2] - 2026-11-14

### Changed
- Docs link to upstream sources (vgi-lint VGI183): the catalog `vgi.doc_md` now
  ends with a "Sources and references" section linking google/pprof and its profile.proto schema.

### Fixed
- Build: drop the unused `pub(crate) use go_cpu2_b64;` re-export (the macro is only
  used inside `meta.rs`), which failed `cargo clippy -D warnings` (`unused import`).

## [0.4.1] - 2026-11-14

### Fixed
- Example queries now show real output for any user, not only inside this repo
  (vgi-lint-check 2.7.0 VGI516/VGI913). They read `data/*.pb.gz` (repo-relative)
  or the placeholder `/profiles/*.pb.gz`, so outside the repo they returned no
  rows. Every function, schema, and catalog example — including the composed
  ones and `vgi.executable_examples` — now decodes an inline base64 copy of the
  committed fixture (`from_base64('...')`); a unit test pins the inline copies
  byte-for-byte to `data/`. The cross-profile top-functions example now sums two
  inline CPU profiles instead of a lateral `glob()` join, and the default-sample-type
  composed example runs on the heap profile.
- README: the quick-start example decodes an inline sample profile, and the
  directory example passes the glob straight to `pprof.stacks(...)` (the
  `glob() f, stacks(f.path)` lateral form does not work for table functions).

## [0.4.0] - 2026-11-12

### Changed
- Ported to VGI protocol 3.4 (vgi.v3 3.3.0): vgi 0.53.2, vgi-rpc 0.32.4.
  Requires the current community `vgi` extension (3.3); 3.2 extensions are
  incompatible. No worker-visible change (the worker has no catalog tables).

## [0.1.1] - 2026-07-21

### Changed
- Publish the worker's build version as the catalog `implementation_version`
  metadata (surfaced by `catalog_catalogs()`) instead of a parameterless
  `pprof_version()` scalar function (vgi-lint VGI328). **Breaking:** the
  `pprof.pprof_version()` scalar is removed; read the version from catalog
  metadata.

### Fixed
- vgi-lint metadata quality back to 100/100 under vgi-lint-check 0.73.0: every
  function and the `main` schema now carry a described `vgi.example_queries`
  JSON list (VGI515), and DuckDB type names in catalog/schema/function docs are
  code-formatted (VGI182).

## [0.1.0] - 2026-06-29

### Added
- Initial release. A VGI worker that decodes pprof (`profile.proto`) profiles
  into SQL rows under the `pprof` catalog.
- Table functions: `stacks` (the headline flattened-stack view), `samples`,
  `functions`, `locations`, `mappings`, and `meta`. Scalar: `pprof_version`.
- Overloaded `src` argument (VARCHAR path / glob, `LIST(VARCHAR)`, or `BLOB`)
  with per-file error capture (`file` + `error` columns).
- Id passthrough across the raw graph views; `mappings.build_id` emitted
  verbatim for downstream symbolication (`vgi-symbols`).
- Vendored Google `profile.proto` (Apache-2.0) compiled with `prost-build`;
  gunzip via `flate2`.
- Golden fixtures (Go CPU + heap, multi-value alloc, unsymbolized native) and
  proptest no-panic fuzzing; haybarn SQLLogic E2E across all transports.
