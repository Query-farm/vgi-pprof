//! `pprof.meta(src)` — one metadata row per profile: sample value-types,
//! sampling period, profile duration/time, and the default sample type.

use arrow_array::RecordBatch;
use arrow_schema::{DataType, SchemaRef};
use vgi::table_function::{TableFunction, TableProducer};
use vgi::{ArgSpec, BindParams, BindResponse, FunctionExample, FunctionMetadata, ProcessParams};
use vgi_rpc::{Result, RpcError};

use pprof_core::ValueType;

use crate::arrow_build as ab;
use crate::source::{self, DecodeUnit};
use crate::table::{commented, schema_with_trailing, OneShot};

const EXECUTABLE_EXAMPLES: &str = r#"[
  {
    "description": "The sample types and duration of a profile.",
    "sql": "SELECT sample_types, duration_nanos, default_sample_type FROM pprof.main.meta('data/go_heap.pb.gz') WHERE error IS NULL"
  },
  {
    "description": "The sampling period and what it counts (read this to convert raw counts to time), plus the wall-clock duration covered.",
    "sql": "SELECT period, period_type, duration_nanos FROM pprof.main.meta('data/go_cpu.pb.gz') WHERE error IS NULL"
  }
]"#;

pub struct MetaTable;

pub fn output_schema() -> SchemaRef {
    schema_with_trailing(vec![
        commented(
            "sample_types",
            ab::list_value_type_type(),
            "The value types each sample carries, in order: a LIST of STRUCT(type VARCHAR, unit \
             VARCHAR) (e.g. [(alloc_objects, count), (alloc_space, bytes)]). The Nth entry names \
             value[N] in pprof.stacks / pprof.samples.",
        ),
        commented(
            "period",
            DataType::Int64,
            "Number of events between sampled occurrences (the sampling period), e.g. the CPU \
             sample interval in nanoseconds.",
        ),
        commented(
            "period_type",
            DataType::Struct(ab::value_type_fields()),
            "The kind of event the period counts, as STRUCT(type VARCHAR, unit VARCHAR) (e.g. \
             (cpu, nanoseconds)); NULL if unset.",
        ),
        commented(
            "duration_nanos",
            DataType::Int64,
            "Wall-clock duration the profile covers, in nanoseconds (0 if not meaningful).",
        ),
        commented(
            "time_nanos",
            DataType::Int64,
            "Collection time as nanoseconds since the Unix epoch (0 if unset).",
        ),
        commented(
            "default_sample_type",
            DataType::Utf8,
            "The preferred sample value type's name (the `type` a viewer defaults to); NULL if \
             unset, in which case clients default to the last sample type.",
        ),
    ])
}

impl TableFunction for MetaTable {
    fn name(&self) -> &str {
        "meta"
    }

    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "Profile Metadata",
            "Decode a pprof profile's metadata into one row: `sample_types` \
             (LIST(STRUCT(type, unit)) describing each value slot), `period`, `period_type` \
             (STRUCT(type, unit)), `duration_nanos`, `time_nanos`, and `default_sample_type`. Use \
             `sample_types` to interpret the `value` LIST in pprof.stacks / pprof.samples (the Nth \
             type names value[N]). `src` may be a path, glob, list, or BLOB; a bad file yields one \
             error row.",
            "pprof profile metadata (one row): `sample_types` (STRUCT(type,unit)[]), `period`, \
             `period_type` (STRUCT(type,unit)), `duration_nanos`, `time_nanos`, \
             `default_sample_type`. `sample_types` names the `value` slots.",
            "pprof, meta, metadata, sample types, sample_type, period, period type, duration, \
             time, default sample type, value types, profiling",
        );
        tags.push((
            "vgi.result_columns_md".into(),
            "| column | type | description |\n\
             |---|---|---|\n\
             | `sample_types` | STRUCT(type VARCHAR, unit VARCHAR)[] | Names each `value` slot. |\n\
             | `period` | BIGINT | Events between samples. |\n\
             | `period_type` | STRUCT(type VARCHAR, unit VARCHAR) | What the period counts. |\n\
             | `duration_nanos` | BIGINT | Profile duration (ns). |\n\
             | `time_nanos` | BIGINT | Collection time (ns since epoch). |\n\
             | `default_sample_type` | VARCHAR | Preferred value type name. |\n\
             | `file` | VARCHAR | Source path (NULL for BLOB input). |\n\
             | `error` | VARCHAR | NULL on success, else the decode error. |"
                .into(),
        ));
        tags.push(("vgi.executable_examples".into(), EXECUTABLE_EXAMPLES.into()));
        FunctionMetadata {
            description: "Decode a pprof profile's metadata (sample types, period, duration)"
                .into(),
            examples: vec![FunctionExample {
                sql: "SELECT sample_types, period, duration_nanos, default_sample_type FROM \
                      pprof.main.meta('heap.pb.gz') WHERE error IS NULL;"
                    .into(),
                description: "Inspect a profile's sample value types and duration.".into(),
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
    let mut sample_types: Vec<Option<Vec<ValueType>>> = Vec::new();
    let mut period: Vec<Option<i64>> = Vec::new();
    let mut period_type: Vec<Option<ValueType>> = Vec::new();
    let mut duration_nanos: Vec<Option<i64>> = Vec::new();
    let mut time_nanos: Vec<Option<i64>> = Vec::new();
    let mut default_sample_type: Vec<Option<String>> = Vec::new();
    let mut file: Vec<Option<String>> = Vec::new();
    let mut error: Vec<Option<String>> = Vec::new();

    for unit in units {
        match &unit.result {
            Ok(decoded) => {
                let m = decoded.meta();
                sample_types.push(Some(m.sample_types));
                period.push(Some(m.period));
                period_type.push(m.period_type);
                duration_nanos.push(Some(m.duration_nanos));
                time_nanos.push(Some(m.time_nanos));
                default_sample_type.push(m.default_sample_type);
                file.push(unit.file.clone());
                error.push(None);
            }
            Err(msg) => {
                sample_types.push(None);
                period.push(None);
                period_type.push(None);
                duration_nanos.push(None);
                time_nanos.push(None);
                default_sample_type.push(None);
                file.push(unit.file.clone());
                error.push(Some(msg.clone()));
            }
        }
    }

    RecordBatch::try_new(
        schema.clone(),
        vec![
            ab::list_value_type_col(&sample_types),
            ab::i64_col(&period),
            ab::value_type_struct_col(&period_type),
            ab::i64_col(&duration_nanos),
            ab::i64_col(&time_nanos),
            ab::str_col(&default_sample_type),
            ab::str_col(&file),
            ab::str_col(&error),
        ],
    )
    .map_err(|e| RpcError::runtime_error(e.to_string()))
}
