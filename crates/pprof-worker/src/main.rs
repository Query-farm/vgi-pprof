//! The `pprof` VGI worker.
//!
//! A standalone binary DuckDB launches and talks to over Apache Arrow IPC
//! (`ATTACH 'pprof' (TYPE vgi, LOCATION '…')`). It decodes **pprof** profiles —
//! the gzip-wrapped `profile.proto` produced by Go, gperftools, Parca,
//! Pyroscope, py-spy, etc. — into rows under the catalog `pprof`, schema `main`,
//! so SRE/perf teams can bulk-diff profiles in SQL:
//!
//! ```sql
//! ATTACH 'pprof' (TYPE vgi, LOCATION './target/release/pprof-worker');
//! SET search_path = 'pprof.main';
//!
//! -- top self-time functions across a directory of CPU profiles
//! SELECT s.frame[1].function AS fn, sum(s.value[2]) AS cpu_ns
//! FROM glob('/profiles/*.pb.gz') f, pprof.stacks(f.path) s
//! WHERE s.error IS NULL
//! GROUP BY 1 ORDER BY 2 DESC LIMIT 20;
//! ```
//!
//! The pure decode/flatten engine lives in the `pprof-core` crate (gunzip +
//! prost-decoded `profile.proto` → row structs); this crate is a thin Arrow
//! adapter: `source` resolves the overloaded `src` argument (path/glob/list/BLOB)
//! with per-file error capture, `arrow_build` turns the row structs into Arrow
//! columns, and `table`/`scalar` register the functions.

mod arrow_build;
mod meta;
mod scalar;
mod source;
mod table;

use vgi::catalog::{CatSchema, CatalogModel};
use vgi::Worker;

/// Worker version string, surfaced by `pprof_version()`.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Catalog + schema metadata (description, provenance) surfaced to DuckDB and
/// the `vgi-lint` metadata-quality linter. The function objects themselves are
/// served from the registered scalars/tables; this only adds catalog/schema-level
/// comments and tags.
fn catalog_metadata(name: &str) -> CatalogModel {
    CatalogModel {
        name: name.to_string(),
        comment: Some(
            "Decode pprof (profile.proto) CPU/heap/alloc/mutex profiles into SQL rows for bulk \
             profile diffing and CI performance-regression gates."
                .to_string(),
        ),
        tags: vec![
            (
                "vgi.title".to_string(),
                "pprof Profile Decoder".to_string(),
            ),
            (
                "vgi.keywords".to_string(),
                crate::meta::keywords_json(
                    "pprof, profile, profile.proto, profiling, flamegraph, CPU profile, heap \
                     profile, alloc, allocation, mutex, contention, Go pprof, gperftools, Parca, \
                     Pyroscope, py-spy, perf regression, stack trace, samples, build id, \
                     symbolization, observability, SRE",
                ),
            ),
            (
                "vgi.doc_llm".to_string(),
                "Decode pprof profiles (the gzip-wrapped profile.proto emitted by Go, gperftools, \
                 Parca, Pyroscope, py-spy, …) into SQL rows so you can bulk-diff profiles and gate \
                 CI on performance regressions. `pprof.stacks(src)` is the headline view: one row \
                 per sample with a LIST(BIGINT) `value` aligned to the sample types, a MAP `labels`, \
                 and a leaf-first LIST(STRUCT(function, filename, line, address)) `frame`, so a \
                 flamegraph diff is GROUP BY frame … SUM(value) with no join. `pprof.samples`, \
                 `pprof.functions`, `pprof.locations`, and `pprof.mappings` expose the raw graph \
                 (ids passed through verbatim so they join), and `pprof.meta` returns one row of \
                 sample types / period / duration. `src` overloads on a VARCHAR path (which may \
                 glob), a LIST(VARCHAR) of paths, or a BLOB of profile bytes; each file is decoded \
                 independently and a malformed/zero-byte profile yields one error row (data columns \
                 NULL, file + error set) instead of aborting. `pprof.mappings.build_id` is emitted \
                 verbatim so unresolved native frames flow to vgi-symbols for symbolization."
                    .to_string(),
            ),
            (
                "vgi.doc_md".to_string(),
                "# pprof\n\nDecode **pprof** profiles — the gzip-wrapped `profile.proto` produced \
                 by Go, gperftools, Parca, Pyroscope, py-spy, and friends — into rows so SRE and \
                 performance teams can **bulk-diff profiles in SQL** and wire CI \
                 performance-regression gates. `go tool pprof` is interactive and single-file; this \
                 worker lets you load hundreds of profiles and ask \"which function regressed across \
                 this deploy?\"\n\nThe headline view is **`pprof.stacks(src)`**: one row per sample \
                 with the call stack pre-resolved — `value` (a `LIST(BIGINT)` aligned to the \
                 profile's sample types), `labels` (a `MAP`), and `frame` (a leaf-first \
                 `LIST(STRUCT(function, filename, line, address))`, inlined frames expanded, \
                 unsymbolized frames keeping their address). A flamegraph diff is then just \
                 `GROUP BY frame … SUM(value)` with no manual join.\n\nThe raw protobuf graph is \
                 also exposed for joins: `pprof.samples` (location ids + values + labels), \
                 `pprof.functions`, `pprof.locations` (line table), and `pprof.mappings` (loaded \
                 binaries with `build_id`), all emitting the profile's original ids verbatim. \
                 `pprof.meta` returns a single row describing the sample value types, sampling \
                 period, and duration.\n\nThe `src` argument overloads on a **path** (which may be \
                 a glob like `/profiles/*.pb.gz`), a **LIST(VARCHAR)** of paths, or a **BLOB** of \
                 profile bytes. Each file is decoded independently, so a malformed or zero-byte \
                 profile produces a single **error row** (every data column NULL, `file` and \
                 `error` set) rather than failing the whole scan. A mapping's `build_id` is passed \
                 through verbatim so the addresses of unsymbolized native frames can be symbolized \
                 downstream by `vgi-symbols`.\n\n**Scalars:** `pprof_version`. **Table functions:** \
                 `stacks`, `samples`, `functions`, `locations`, `mappings`, `meta`."
                    .to_string(),
            ),
            // Fixed agent-suitability suite run by `vgi-lint simulate`. The
            // file-based tasks use repo-relative `data/...` fixture paths, so run
            // `vgi-lint simulate` from the repo root where the fixtures live.
            (
                "vgi.agent_test_tasks".to_string(),
                crate::meta::agent_test_tasks_json(&[
                    (
                        "top_cpu_function",
                        "The file data/go_cpu.pb.gz is a Go CPU profile. Which leaf function has \
                         the most total CPU time? CPU nanoseconds is the second sample value. \
                         Return one row with a column named fn and a column named cpu_ns.",
                        "SELECT frame[1].function AS fn, sum(value[2]) AS cpu_ns \
                         FROM pprof.main.stacks('data/go_cpu.pb.gz') WHERE error IS NULL \
                         GROUP BY 1 ORDER BY 2 DESC LIMIT 1",
                    ),
                    (
                        "heap_sample_types",
                        "data/go_heap.pb.gz is a Go heap profile. How many distinct sample value \
                         types does it carry? Return one row with a single column named n.",
                        "SELECT len(sample_types) AS n FROM pprof.main.meta('data/go_heap.pb.gz') \
                         WHERE error IS NULL",
                    ),
                    (
                        "native_build_ids",
                        "data/native.pb.gz is an unsymbolized native profile. List the build ids of \
                         its mappings that have one, so they can be symbolized. Return a column \
                         named build_id.",
                        "SELECT build_id FROM pprof.main.mappings('data/native.pb.gz') \
                         WHERE error IS NULL AND build_id IS NOT NULL ORDER BY build_id",
                    ),
                    (
                        "regression_diff",
                        "Compare two CPU profiles data/go_cpu.pb.gz (baseline) and \
                         data/go_cpu2.pb.gz (candidate): for the busiest leaf function in the \
                         candidate, return its name and the CPU-nanosecond delta (candidate minus \
                         baseline). Return columns fn and delta.",
                        "WITH b AS (SELECT frame[1].function AS fn, sum(value[2]) AS ns \
                         FROM pprof.main.stacks('data/go_cpu.pb.gz') WHERE error IS NULL GROUP BY 1), \
                         c AS (SELECT frame[1].function AS fn, sum(value[2]) AS ns \
                         FROM pprof.main.stacks('data/go_cpu2.pb.gz') WHERE error IS NULL GROUP BY 1) \
                         SELECT c.fn AS fn, c.ns - COALESCE(b.ns, 0) AS delta \
                         FROM c LEFT JOIN b USING (fn) ORDER BY c.ns DESC LIMIT 1",
                    ),
                    (
                        "worker_version",
                        "What version of the pprof worker is currently running? Return a single \
                         row with one column named version.",
                        "SELECT pprof.main.pprof_version() AS version",
                    ),
                ]),
            ),
            // VGI151/VGI506 representative example queries at the catalog level.
            (
                "vgi.example_queries".to_string(),
                "SELECT s.frame[1].function AS fn, sum(s.value[2]) AS cpu_ns \
                 FROM glob('/profiles/*.pb.gz') f, pprof.main.stacks(f.path) s \
                 WHERE s.error IS NULL GROUP BY 1 ORDER BY 2 DESC LIMIT 20;\n\
                 SELECT * FROM pprof.main.meta('cpu.pb.gz');\n\
                 SELECT sample_id, frame[1].function AS leaf, value \
                 FROM pprof.main.stacks('cpu.pb.gz') WHERE error IS NULL;\n\
                 SELECT mapping_id, filename, build_id FROM pprof.main.mappings('native.pb.gz') \
                 WHERE build_id IS NOT NULL;\n\
                 SELECT pprof.main.pprof_version();"
                    .to_string(),
            ),
            ("vgi.author".to_string(), "Query.Farm".to_string()),
            (
                "vgi.copyright".to_string(),
                "Copyright 2026 Query Farm LLC - https://query.farm".to_string(),
            ),
            ("vgi.license".to_string(), "MIT".to_string()),
            (
                "vgi.support_contact".to_string(),
                "https://github.com/Query-farm/vgi-pprof/issues".to_string(),
            ),
            (
                "vgi.support_policy_url".to_string(),
                "https://github.com/Query-farm/vgi-pprof/blob/main/README.md".to_string(),
            ),
        ],
        source_url: Some("https://github.com/Query-farm/vgi-pprof".to_string()),
        schemas: vec![CatSchema {
            name: "main".to_string(),
            comment: Some(
                "pprof profile decoding functions: stacks, samples, functions, locations, \
                 mappings, meta."
                    .to_string(),
            ),
            tags: vec![
                ("vgi.title".to_string(), "pprof — main".to_string()),
                (
                    "vgi.keywords".to_string(),
                    crate::meta::keywords_json(
                        "pprof, stacks, samples, functions, locations, mappings, meta, profile, \
                         flamegraph, profiling, regression, build id, symbolization",
                    ),
                ),
                // VGI123 classifying tags (bare keys: domain/category/topic).
                ("domain".to_string(), "observability".to_string()),
                ("category".to_string(), "profiling".to_string()),
                ("topic".to_string(), "pprof-profiles".to_string()),
                (
                    "vgi.doc_llm".to_string(),
                    "Functions to decode a pprof profile into rows: `stacks` (flattened, \
                     flamegraph-ready samples — the headline view), `samples` (raw samples), \
                     `functions`, `locations`, and `mappings` (the protobuf graph with ids passed \
                     through verbatim for joins), and `meta` (one row of sample types, period, and \
                     duration). The `src` argument is a path, a glob, a LIST(VARCHAR), or a BLOB; a \
                     bad file becomes one error row."
                        .to_string(),
                ),
                (
                    "vgi.doc_md".to_string(),
                    "The single schema for the `pprof` worker — the catalog name matches the \
                     `ATTACH` name, so qualify calls as `pprof.main.<fn>(...)`. It holds the six \
                     pprof table functions — `stacks` (the headline flattened-stack view), \
                     `samples`, `functions`, `locations`, `mappings`, and `meta` — plus the \
                     `pprof_version` scalar. Every table takes an overloaded `src` (path / glob / \
                     LIST(VARCHAR) / BLOB) and appends `file` and `error` columns for per-file \
                     provenance and error capture."
                        .to_string(),
                ),
                // VGI506 representative example queries for the schema.
                (
                    "vgi.example_queries".to_string(),
                    "SELECT s.frame[1].function AS fn, sum(s.value[2]) AS cpu_ns \
                     FROM glob('/profiles/*.pb.gz') f, pprof.main.stacks(f.path) s \
                     WHERE s.error IS NULL GROUP BY 1 ORDER BY 2 DESC LIMIT 20;\n\
                     SELECT * FROM pprof.main.meta('data/go_heap.pb.gz');\n\
                     SELECT * FROM pprof.main.mappings('data/native.pb.gz') WHERE build_id IS NOT NULL;\n\
                     SELECT pprof.main.pprof_version();"
                        .to_string(),
                ),
            ],
            views: Vec::new(),
            macros: Vec::new(),
            tables: Vec::new(),
        }],
        ..Default::default()
    }
}

fn main() {
    // Logs MUST go to stderr — stdout is the Arrow-IPC channel.
    let _ = env_logger::Builder::from_env(env_logger::Env::default().filter_or("VGI_LOG", "info"))
        .format_timestamp_millis()
        .try_init();

    // The catalog name DuckDB sees in `ATTACH 'pprof' (TYPE vgi, …)`. Default to
    // `pprof`, but honor an explicit override so a test harness can rename it.
    if std::env::var_os("VGI_WORKER_CATALOG_NAME").is_none() {
        std::env::set_var("VGI_WORKER_CATALOG_NAME", "pprof");
    }
    let catalog_name =
        std::env::var("VGI_WORKER_CATALOG_NAME").unwrap_or_else(|_| "pprof".to_string());

    let mut worker = Worker::new();
    scalar::register(&mut worker);
    table::register(&mut worker);
    worker.set_catalog(catalog_metadata(&catalog_name));
    worker.run();
}
