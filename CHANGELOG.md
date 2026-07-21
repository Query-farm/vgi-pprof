# Changelog

All notable changes to `vgi-pprof` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/), and the project adheres to
[Semantic Versioning](https://semver.org/).

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
