# Changelog

All notable changes to `vgi-pprof` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/), and the project adheres to
[Semantic Versioning](https://semver.org/).

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
