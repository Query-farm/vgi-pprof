//! `pprof.samples(src)` — the raw samples view: location ids + per-type values
//! + labels, with no location/function resolution applied (join to `locations`).

use arrow_array::RecordBatch;
use arrow_schema::{DataType, SchemaRef};
use vgi::table_function::{TableFunction, TableProducer};
use vgi::{ArgSpec, BindParams, BindResponse, FunctionExample, FunctionMetadata, ProcessParams};
use vgi_rpc::{Result, RpcError};

use crate::arrow_build as ab;
use crate::source::{self, DecodeUnit};
use crate::table::{commented, schema_with_trailing, OneShot};

const EXECUTABLE_EXAMPLES: &str = r#"[
  {
    "description": "Raw samples with their leaf location id.",
    "sql": "SELECT sample_id, location_ids[1] AS leaf_loc, value FROM pprof.main.samples('data/go_cpu.pb.gz') WHERE error IS NULL"
  },
  {
    "description": "Inspect the per-sample labels (e.g. a heap profile's size_class tag) carried alongside the values.",
    "sql": "SELECT sample_id, labels, value FROM pprof.main.samples('data/alloc.pb.gz') WHERE error IS NULL LIMIT 5"
  }
]"#;

pub struct Samples;

pub fn output_schema() -> SchemaRef {
    schema_with_trailing(vec![
        commented(
            "sample_id",
            DataType::Int64,
            "1-based index of the sample within its profile; joins to pprof.stacks.sample_id.",
        ),
        commented(
            "location_ids",
            ab::list_u64_type(),
            "The sample's location ids, leaf first (location_ids[1] is the leaf). Each joins to \
             pprof.locations.location_id.",
        ),
        commented(
            "value",
            ab::list_i64_type(),
            "The sample's measured values, one BIGINT per meta.sample_types entry, in order.",
        ),
        commented(
            "labels",
            ab::map_type(),
            "Sample labels as a MAP(VARCHAR, VARCHAR); duplicate keys de-duplicated (first wins).",
        ),
    ])
}

impl TableFunction for Samples {
    fn name(&self) -> &str {
        "samples"
    }

    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "Raw Samples",
            "Decode a pprof profile's raw samples: one row per sample with `location_ids` \
             (LIST(UBIGINT), leaf first), `value` (LIST(BIGINT) aligned to meta.sample_types), and \
             `labels` (MAP). No resolution is applied — join `location_ids` to pprof.locations and \
             onward to pprof.functions, or use pprof.stacks for the pre-resolved view. `src` may be \
             a path, glob, list, or BLOB; a bad file yields one error row.",
            "Raw pprof samples: `sample_id`, `location_ids` (UBIGINT[], leaf first), `value` \
             (BIGINT[]), and `labels` (MAP). Join `location_ids` to `locations`, or use `stacks` \
             for resolved frames.",
            "pprof, samples, raw samples, location ids, values, labels, stack, profiling, join",
        );
        tags.push((
            "vgi.result_columns_md".into(),
            "| column | type | description |\n\
             |---|---|---|\n\
             | `sample_id` | BIGINT | 1-based sample index. |\n\
             | `location_ids` | UBIGINT[] | Location ids, leaf first. |\n\
             | `value` | BIGINT[] | Values aligned to `meta.sample_types`. |\n\
             | `labels` | MAP(VARCHAR,VARCHAR) | Sample labels. |\n\
             | `file` | VARCHAR | Source path (NULL for BLOB input). |\n\
             | `error` | VARCHAR | NULL on success, else the decode error. |"
                .into(),
        ));
        tags.push(("vgi.executable_examples".into(), EXECUTABLE_EXAMPLES.into()));
        FunctionMetadata {
            description: "Decode a pprof profile's raw samples (location ids + values + labels)"
                .into(),
            examples: vec![FunctionExample {
                sql: "SELECT sample_id, location_ids[1] AS leaf_loc, value FROM \
                      pprof.main.samples('data/go_cpu.pb.gz') WHERE error IS NULL;"
                    .into(),
                description: "Raw samples with their leaf location id.".into(),
                expected_output: None,
            }],
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
    let mut location_ids: Vec<Option<Vec<u64>>> = Vec::new();
    let mut value: Vec<Option<Vec<i64>>> = Vec::new();
    let mut labels: Vec<Option<Vec<(String, String)>>> = Vec::new();
    let mut file: Vec<Option<String>> = Vec::new();
    let mut error: Vec<Option<String>> = Vec::new();

    for unit in units {
        match &unit.result {
            Ok(decoded) => {
                for s in decoded.samples() {
                    sample_id.push(Some(s.sample_id));
                    location_ids.push(Some(s.location_ids));
                    value.push(Some(s.value));
                    labels.push(Some(s.labels));
                    file.push(unit.file.clone());
                    error.push(None);
                }
            }
            Err(msg) => {
                sample_id.push(None);
                location_ids.push(None);
                value.push(None);
                labels.push(None);
                file.push(unit.file.clone());
                error.push(Some(msg.clone()));
            }
        }
    }

    RecordBatch::try_new(
        schema.clone(),
        vec![
            ab::i64_col(&sample_id),
            ab::list_u64_col(&location_ids),
            ab::list_i64_col(&value),
            ab::map_col(&labels),
            ab::str_col(&file),
            ab::str_col(&error),
        ],
    )
    .map_err(|e| RpcError::runtime_error(e.to_string()))
}
