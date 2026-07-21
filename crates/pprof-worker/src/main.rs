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
mod source;
mod table;

use vgi::catalog::{CatSchema, CatView, CatalogModel};
use vgi::Worker;

/// Worker build version string, published as the catalog's
/// `implementation_version` metadata.
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
                "Decode pprof profiles — the gzip-wrapped profile.proto emitted by Go, gperftools, \
                 Parca, Pyroscope, and py-spy — into SQL rows over Arrow so you can bulk-diff many \
                 profiles at once and gate CI on performance regressions, which the interactive, \
                 single-file `go tool pprof` can't do. The headline is a flattened, flamegraph-ready \
                 view: one row per sample carrying its per-type measured values (a list of BIGINTs \
                 aligned to the profile's sample value types), its labels as a `MAP`, and its call \
                 stack pre-resolved leaf-first as a list of frame structs (inlined frames expanded, \
                 unsymbolized frames keeping their raw address) — so a flamegraph diff becomes a \
                 GROUP BY over frames with a SUM and no manual join. The underlying protobuf graph \
                 is also exposed with the profile's original ids passed through verbatim so rows \
                 join cleanly, plus a one-row profile summary (sample value types, sampling period, \
                 duration). Build ids are emitted verbatim so the addresses of unsymbolized native \
                 frames can be resolved downstream by a symbolizer. Reach for this whenever you \
                 need to compare, aggregate, or regression-gate profiles across a fleet, a deploy, \
                 or a CI run rather than eyeballing one at a time. Each source is decoded \
                 independently — a path (which may be a glob), a list of paths, or inline profile \
                 bytes — and a missing, empty, or malformed profile becomes one error row (data \
                 columns NULL) instead of aborting the scan."
                    .to_string(),
            ),
            (
                "vgi.doc_md".to_string(),
                "# pprof\n\nDecode **pprof** profiles — the gzip-wrapped `profile.proto` produced \
                 by Go, gperftools, Parca, Pyroscope, py-spy, and friends — into rows so SRE and \
                 performance teams can **bulk-diff profiles in SQL** and wire up CI \
                 performance-regression gates. `go tool pprof` is interactive and single-file; this \
                 worker lets you load hundreds of profiles at once and ask \"which function \
                 regressed across this deploy?\"\n\nThe headline is a **flattened, flamegraph-ready \
                 view**: one row per sample with its call stack already resolved — the per-type \
                 measured values (a `BIGINT[]` aligned to the profile's sample value types), the \
                 sample labels as a `MAP`, and the call stack as a leaf-first list of `(function, \
                 filename, line, address)` frames, with inlined frames expanded and unsymbolized \
                 frames keeping their raw address. A flamegraph diff across many profiles then \
                 needs no manual join.\n\nThe underlying `profile.proto` graph is also exposed for \
                 ad-hoc joins, with the profile's original ids passed through verbatim, alongside a \
                 one-row profile summary (sample value types, sampling period, duration). A loaded \
                 binary's build id is emitted verbatim so the addresses of unsymbolized native \
                 frames can be resolved by a downstream symbolizer.\n\nReach for this worker \
                 whenever you need to **compare or aggregate profiles at scale** — across a fleet, \
                 a deploy, or a CI run — rather than inspecting one at a time. Each source is \
                 decoded independently — a **path** (which may be a glob like \
                 `/profiles/*.pb.gz`), a **list of paths**, or **inline profile bytes** — so a \
                 malformed or zero-byte profile produces a single **error row** (every data column \
                 NULL) rather than failing the whole scan."
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
                        "sample_count",
                        "data/go_cpu.pb.gz is a Go CPU profile. How many samples (recorded stack \
                         traces) does it contain? Return one row with a single column named n.",
                        "SELECT count(*) AS n FROM pprof.main.samples('data/go_cpu.pb.gz') \
                         WHERE error IS NULL",
                    ),
                    (
                        "function_table_size",
                        "How many rows does the function table of data/go_cpu.pb.gz have (the \
                         number of distinct symbolized functions in the profile)? Return one row \
                         with a single column named n.",
                        "SELECT count(*) AS n FROM pprof.main.functions('data/go_cpu.pb.gz') \
                         WHERE error IS NULL",
                    ),
                    (
                        "deepest_inlining",
                        "In data/go_cpu.pb.gz, across all locations, what is the greatest number \
                         of line-table entries on any single location (the deepest inlining the \
                         compiler folded into one instruction address)? Return one row with a \
                         single column named max_inline.",
                        "SELECT max(len(lines)) AS max_inline \
                         FROM pprof.main.locations('data/go_cpu.pb.gz') WHERE error IS NULL",
                    ),
                    (
                        "cpu_value_index",
                        "Using the worker's built-in reference guide of pprof sample value types, \
                         which value index carries CPU time measured in nanoseconds for a Go 'cpu' \
                         profile? Return one row with a single column named value_index.",
                        "SELECT value_index FROM pprof.main.sample_type_guide \
                         WHERE producer = 'Go' AND profile_kind = 'cpu' AND unit = 'nanoseconds'",
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
                 WHERE build_id IS NOT NULL;"
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
        // Publish the running build version as catalog metadata (surfaced via
        // catalog_catalogs) rather than as a parameterless pprof_version() scalar
        // (VGI328).
        implementation_version: Some(version().to_string()),
        schemas: vec![CatSchema {
            name: "main".to_string(),
            comment: Some(
                "Decode pprof profiles into SQL rows: flattened flamegraph-ready stacks, the raw \
                 profile.proto graph, and profile metadata."
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
                // VGI413/408-412 navigation registry: an ordered list of the
                // sections objects are grouped under. Each object carries a
                // matching `vgi.category` (see crate::meta::object_tags).
                (
                    "vgi.categories".to_string(),
                    "[\
                     {\"name\":\"Flattened stacks\",\"description\":\"The headline view: \
                     flamegraph-ready rows with each sample's call stack pre-resolved, for \
                     diffing profiles in SQL without a manual join.\"},\
                     {\"name\":\"Raw profile graph\",\"description\":\"The raw profile.proto graph \
                     — samples and the location, function, and mapping tables they reference — with \
                     original ids passed through verbatim for ad-hoc joins.\"},\
                     {\"name\":\"Profile metadata\",\"description\":\"Profile-level summary and \
                     worker introspection: the sample value types, sampling period and duration, \
                     and the running worker version.\"}\
                     ]"
                        .to_string(),
                ),
                (
                    "vgi.doc_llm".to_string(),
                    "Decode a pprof profile into SQL rows. The headline is a flattened, \
                     flamegraph-ready view — one row per sample with its per-type values (a list of \
                     BIGINTs aligned to the profile's sample value types), its labels as a `MAP`, \
                     and its call stack pre-resolved leaf-first as frame structs — so a flamegraph \
                     diff is a GROUP BY over frames with a SUM and no join. The underlying protobuf \
                     graph \
                     is also exposed for ad-hoc joins with the profile's original ids passed \
                     through verbatim, alongside a one-row summary of the sample value types, \
                     sampling period, and duration. The profile source is a path, a glob, a list of \
                     paths, or inline bytes; a bad file becomes one error row rather than aborting \
                     the scan."
                        .to_string(),
                ),
                (
                    "vgi.doc_md".to_string(),
                    "## pprof · main\n\nThe single schema for the `pprof` worker — the catalog \
                     name matches the `ATTACH` name, so calls qualify as \
                     `pprof.main.<fn>(...)`.\n\nIt decodes a pprof profile into SQL rows across \
                     three groups: a flattened, flamegraph-ready view of samples with their call \
                     stacks pre-resolved (the headline), the raw `profile.proto` graph for ad-hoc \
                     joins, and a one-row profile summary of the sample value types, sampling \
                     period, and duration.\n\nEvery table takes the same overloaded profile \
                     source — a path, a glob, a list of paths, or inline bytes — and appends \
                     `file` and `error` columns for per-file provenance and error capture, so a \
                     bad file in a glob becomes one error row instead of aborting the scan."
                        .to_string(),
                ),
                // VGI506/VGI515 representative example queries for the schema, as a
                // described JSON list of {description, sql}.
                (
                    "vgi.example_queries".to_string(),
                    crate::meta::example_queries_json(&[
                        (
                            "Top self-time functions across a directory of CPU profiles.".into(),
                            "SELECT s.frame[1].function AS fn, sum(s.value[2]) AS cpu_ns \
                             FROM glob('/profiles/*.pb.gz') f, pprof.main.stacks(f.path) s \
                             WHERE s.error IS NULL GROUP BY 1 ORDER BY 2 DESC LIMIT 20"
                                .into(),
                        ),
                        (
                            "A heap profile's sample value types, sampling period, and duration."
                                .into(),
                            "SELECT sample_types, period, duration_nanos \
                             FROM pprof.main.meta('data/go_heap.pb.gz') WHERE error IS NULL"
                                .into(),
                        ),
                        (
                            "Mappings that carry a build id (the binaries still to symbolize)."
                                .into(),
                            "SELECT mapping_id, filename, build_id \
                             FROM pprof.main.mappings('data/native.pb.gz') \
                             WHERE error IS NULL AND build_id IS NOT NULL"
                                .into(),
                        ),
                    ]),
                ),
            ],
            views: vec![sample_type_guide_view()],
            macros: Vec::new(),
            tables: Vec::new(),
        }],
        ..Default::default()
    }
}

/// A small, browsable reference view (`pprof.main.sample_type_guide`) that maps
/// the common `(producer, profile_kind, value_index)` conventions to the sample
/// type and unit each `value[N]` slot carries. It is backed entirely by a
/// `VALUES` list, so it scans instantly with no file, network, or credential —
/// and it gives an agent a browsable table to read *before* it has to guess
/// which `value` index to sum (VGI146), e.g. that Go CPU time is `value[2]` in
/// nanoseconds. The authoritative per-profile answer is always
/// `pprof.main.meta(src).sample_types`; this view is the cross-profile cheat
/// sheet for the widespread Go/runtime conventions.
fn sample_type_guide_view() -> CatView {
    // NOTE: keep these rows consistent with the pprof/runtime conventions the
    // tables document (see the `value` column comment on pprof.stacks/samples).
    let definition = "SELECT * FROM (VALUES \
        ('Go', 'cpu', 1, 'samples', 'count', 'Number of CPU samples that landed on this stack'), \
        ('Go', 'cpu', 2, 'cpu', 'nanoseconds', 'CPU time attributed to this stack'), \
        ('Go', 'heap', 1, 'alloc_objects', 'count', 'Objects allocated since the process started'), \
        ('Go', 'heap', 2, 'alloc_space', 'bytes', 'Bytes allocated since the process started'), \
        ('Go', 'heap', 3, 'inuse_objects', 'count', 'Objects still live at profile time'), \
        ('Go', 'heap', 4, 'inuse_space', 'bytes', 'Bytes still live at profile time'), \
        ('Go', 'allocs', 1, 'alloc_objects', 'count', 'Objects allocated since the process started'), \
        ('Go', 'allocs', 2, 'alloc_space', 'bytes', 'Bytes allocated since the process started'), \
        ('Go', 'block', 1, 'contentions', 'count', 'Blocking events observed'), \
        ('Go', 'block', 2, 'delay', 'nanoseconds', 'Time spent blocked'), \
        ('Go', 'mutex', 1, 'contentions', 'count', 'Mutex contention events observed'), \
        ('Go', 'mutex', 2, 'delay', 'nanoseconds', 'Time spent waiting on the mutex')) \
        AS t(producer, profile_kind, value_index, sample_type, unit, meaning)"
        .to_string();

    let example_queries =
        "[{\"description\":\"Look up which value slot holds CPU time for a Go CPU \
        profile (and in what unit).\",\"sql\":\"SELECT value_index, sample_type, unit FROM \
        pprof.main.sample_type_guide WHERE producer = 'Go' AND profile_kind = 'cpu' ORDER BY \
        value_index\"}]"
            .to_string();

    let mut tags = crate::meta::object_tags(
        "pprof Value-Slot Reference",
        "A browsable reference table mapping common pprof `(producer, profile_kind, value_index)` \
         conventions to the `sample_type` and `unit` that each `value[N]` slot carries — e.g. for a \
         Go 'cpu' profile value[1] is samples/count and value[2] is cpu/nanoseconds. Read it to \
         know which `value` index to sum in pprof.stacks / pprof.samples without decoding a file \
         first; the authoritative per-profile answer is pprof.meta(src).sample_types. Backed by a \
         VALUES list, so it scans with no file, network, or credential.",
        "Reference table of common pprof sample value-type conventions: `producer`, \
         `profile_kind`, `value_index`, `sample_type`, `unit`, `meaning`. Tells you which \
         `value[N]` slot to read (e.g. Go CPU time is value[2], nanoseconds).",
        "pprof, sample type, sample_type, value index, cpu, nanoseconds, alloc, heap, inuse, \
         mutex, block, contentions, delay, units, reference, guide, conventions",
        "Profile metadata",
    );
    // VGI123 classifying tag (bare keys, matching the schema's domain/topic
    // vocabulary) — object_tags only sets the navigation `vgi.category`.
    tags.push(("domain".to_string(), "observability".to_string()));
    tags.push(("topic".to_string(), "pprof-profiles".to_string()));
    tags.push(("vgi.example_queries".to_string(), example_queries));

    CatView {
        name: "sample_type_guide".to_string(),
        definition,
        comment: Some(
            "Reference table of common pprof sample value-type conventions: which sample_type and \
             unit each value[N] slot carries per (producer, profile_kind). VALUES-backed, so it \
             scans with no file or credential."
                .to_string(),
        ),
        tags,
        column_comments: vec![
            (
                "producer".to_string(),
                "The profiler/runtime that emitted the profile (e.g. 'Go').".to_string(),
            ),
            (
                "profile_kind".to_string(),
                "The kind of profile within that producer (e.g. 'cpu', 'heap', 'allocs', 'mutex', \
                 'block')."
                    .to_string(),
            ),
            (
                "value_index".to_string(),
                "1-based position of this value in the sample's `value` list (value[value_index] \
                 in pprof.stacks / pprof.samples)."
                    .to_string(),
            ),
            (
                "sample_type".to_string(),
                "The value type's name at this index, matching meta.sample_types[value_index].type."
                    .to_string(),
            ),
            (
                "unit".to_string(),
                "The unit the value is measured in (e.g. 'count', 'bytes', 'nanoseconds')."
                    .to_string(),
            ),
            (
                "meaning".to_string(),
                "A plain-language description of what this value slot counts.".to_string(),
            ),
        ],
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
    table::register(&mut worker);
    worker.set_catalog(catalog_metadata(&catalog_name));
    worker.run();
}
