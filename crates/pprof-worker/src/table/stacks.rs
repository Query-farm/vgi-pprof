//! `pprof.stacks(src)` — the headline flattened-stack view: per-value-type
//! values + pre-resolved frames in one row, so a flamegraph diff is a
//! `GROUP BY frame … value` with no manual location/function join.

use arrow_array::RecordBatch;
use arrow_schema::{DataType, SchemaRef};
use vgi::table_function::{TableFunction, TableProducer};
use vgi::{ArgSpec, BindParams, BindResponse, FunctionMetadata, ProcessParams};
use vgi_rpc::{Result, RpcError};

use pprof_core::Frame;

use crate::arrow_build as ab;
use crate::source::{self, DecodeUnit};
use crate::table::{commented, schema_with_trailing, OneShot};

const EXECUTABLE_EXAMPLES: &str = r#"[
  {
    "description": "Top self-time functions across a directory of CPU profiles.",
    "sql": "SELECT s.frame[1].function AS fn, sum(s.value[2]) AS cpu_ns FROM glob('/profiles/*.pb.gz') f, pprof.main.stacks(f.path) s WHERE s.error IS NULL GROUP BY 1 ORDER BY 2 DESC LIMIT 20"
  },
  {
    "description": "Flatten one profile's stacks (leaf frame, values aligned to meta.sample_types).",
    "sql": "SELECT sample_id, frame[1].function AS leaf, value FROM pprof.main.stacks('data/go_cpu.pb.gz') WHERE error IS NULL"
  }
]"#;

pub struct Stacks;

pub fn output_schema() -> SchemaRef {
    schema_with_trailing(vec![
        commented(
            "sample_id",
            DataType::Int64,
            "1-based index of the sample within its profile; joins to pprof.samples.sample_id.",
        ),
        commented(
            "value",
            ab::list_i64_type(),
            "The sample's measured values, one BIGINT per entry of meta.sample_types, in the same \
             order (e.g. [alloc_objects, alloc_space] for a heap profile). Index the type you want.",
        ),
        commented(
            "labels",
            ab::map_type(),
            "Sample labels as a MAP(VARCHAR, VARCHAR): string labels verbatim, numeric labels \
             rendered as the number (with unit suffix when present). Duplicate keys are \
             de-duplicated (first wins).",
        ),
        commented(
            "frame",
            ab::list_frame_type(),
            "The call stack, leaf first: a LIST of STRUCT(function, filename, line, address). \
             Inlined frames are expanded in place; an unsymbolized frame has NULL function/filename \
             but keeps its address. frame[1] is the leaf (self) frame.",
        ),
    ])
}

impl TableFunction for Stacks {
    fn name(&self) -> &str {
        "stacks"
    }

    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "Flattened Stacks (headline)",
            "Flatten a pprof profile into one row per sample with the call stack pre-resolved: \
             `value` is a `LIST(BIGINT)` aligned to meta.sample_types, `labels` is a \
             `MAP(VARCHAR, VARCHAR)`, and `frame` is a \
             `LIST(STRUCT(function, filename, line, address))` ordered leaf-first (inlined frames \
             expanded, unsymbolized frames keep their address). This is the 90% view: a flamegraph \
             diff across many profiles is a GROUP BY frame … SUM(value) with no manual \
             location/function join. `src` may be a file path, a glob, a list of paths, or inline \
             `BLOB` bytes; a bad file yields one error row.",
            "Flatten a pprof profile into flamegraph-ready rows: `sample_id`, `value` \
             (a `LIST(BIGINT)` aligned to the sample types), `labels` (a `MAP`), and `frame` \
             (a `LIST(STRUCT(function, filename, line, address))`, leaf first). Use it to diff \
             profiles in SQL.",
            "pprof, stacks, flamegraph, stack trace, profile diff, regression, self time, cpu, \
             flatten, frames, leaf, samples, profiling",
            "Flattened stacks",
        );
        let mut cols = vec![
            (
                "sample_id",
                "BIGINT",
                "1-based index of the sample within its profile; joins to pprof.samples.sample_id.",
            ),
            (
                "value",
                "BIGINT[]",
                "The sample's measured values, one BIGINT per meta.sample_types entry, in that \
                 order (e.g. value[2] is cpu/nanoseconds for a Go CPU profile).",
            ),
            (
                "labels",
                "MAP(VARCHAR, VARCHAR)",
                "Sample labels; duplicate keys are de-duplicated (first wins).",
            ),
            (
                "frame",
                "STRUCT(function VARCHAR, filename VARCHAR, line BIGINT, address UBIGINT)[]",
                "The call stack, leaf first; inlined frames expanded, unsymbolized frames keep \
                 their address. frame[1] is the leaf (self) frame.",
            ),
        ];
        cols.extend(crate::meta::trailing_result_columns());
        tags.push((
            "vgi.result_columns_schema".into(),
            crate::meta::result_columns_schema(&cols),
        ));
        tags.push(("vgi.executable_examples".into(), EXECUTABLE_EXAMPLES.into()));
        let (examples, example_queries) = crate::meta::described_examples(vec![
            (
                "Flatten a profile passed inline as BLOB bytes (self-contained): each sample's \
                 leaf frame and per-type values."
                    .into(),
                format!(
                    "SELECT sample_id, frame[1].function AS leaf, value \
                     FROM pprof.main.stacks(from_base64('{}')) WHERE error IS NULL;",
                    crate::meta::GO_CPU_B64
                ),
            ),
            (
                "Top self-time functions in a CPU profile (value[2] = cpu/nanoseconds), read \
                 from a file path."
                    .into(),
                "SELECT frame[1].function AS fn, sum(value[2]) AS cpu_ns FROM \
                 pprof.main.stacks('data/go_cpu.pb.gz') WHERE error IS NULL \
                 GROUP BY 1 ORDER BY 2 DESC LIMIT 20;"
                    .into(),
            ),
        ]);
        tags.push(("vgi.example_queries".into(), example_queries));
        FunctionMetadata {
            description: "Flatten a pprof profile into flamegraph-ready stack rows".into(),
            examples,
            tags,
            ..Default::default()
        }
    }

    fn argument_specs(&self) -> Vec<ArgSpec> {
        vec![source::src_arg_spec()]
    }

    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse {
            output_schema: output_schema(),
            opaque_data: Vec::new(),
        })
    }

    fn producer(&self, params: &ProcessParams) -> Result<Box<dyn TableProducer>> {
        let units = source::resolve(&params.arguments, 0)?;
        Ok(Box::new(OneShot::new(
            params.output_schema.clone(),
            units,
            build,
        )))
    }
}

fn build(units: &[DecodeUnit], schema: &SchemaRef) -> Result<RecordBatch> {
    let mut sample_id: Vec<Option<i64>> = Vec::new();
    let mut value: Vec<Option<Vec<i64>>> = Vec::new();
    let mut labels: Vec<Option<Vec<(String, String)>>> = Vec::new();
    let mut frame: Vec<Option<Vec<Frame>>> = Vec::new();
    let mut file: Vec<Option<String>> = Vec::new();
    let mut error: Vec<Option<String>> = Vec::new();

    for unit in units {
        match &unit.result {
            Ok(decoded) => {
                for s in decoded.stacks() {
                    sample_id.push(Some(s.sample_id));
                    value.push(Some(s.value));
                    labels.push(Some(s.labels));
                    frame.push(Some(s.frames));
                    file.push(unit.file.clone());
                    error.push(None);
                }
            }
            Err(msg) => {
                sample_id.push(None);
                value.push(None);
                labels.push(None);
                frame.push(None);
                file.push(unit.file.clone());
                error.push(Some(msg.clone()));
            }
        }
    }

    RecordBatch::try_new(
        schema.clone(),
        vec![
            ab::i64_col(&sample_id),
            ab::list_i64_col(&value),
            ab::map_col(&labels),
            ab::list_frame_col(&frame),
            ab::str_col(&file),
            ab::str_col(&error),
        ],
    )
    .map_err(|e| RpcError::runtime_error(e.to_string()))
}
