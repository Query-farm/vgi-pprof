//! `pprof.locations(src)` — the location table: location_id → address /
//! mapping_id / lines (LIST(STRUCT(function_id, line))), ids verbatim.

use arrow_array::RecordBatch;
use arrow_schema::{DataType, SchemaRef};
use vgi::table_function::{TableFunction, TableProducer};
use vgi::{ArgSpec, BindParams, BindResponse, FunctionExample, FunctionMetadata, ProcessParams};
use vgi_rpc::{Result, RpcError};

use pprof_core::LineRef;

use crate::arrow_build as ab;
use crate::source::{self, DecodeUnit};
use crate::table::{commented, schema_with_trailing, OneShot};

const EXECUTABLE_EXAMPLES: &str = r#"[
  {
    "description": "Locations with their line table (inlined frames).",
    "sql": "SELECT location_id, address, mapping_id, lines FROM pprof.main.locations('data/go_cpu.pb.gz') WHERE error IS NULL ORDER BY location_id LIMIT 10"
  }
]"#;

pub struct Locations;

pub fn output_schema() -> SchemaRef {
    schema_with_trailing(vec![
        commented(
            "location_id",
            DataType::UInt64,
            "The location's unique id within the profile (verbatim); referenced by \
             pprof.samples.location_ids.",
        ),
        commented(
            "address",
            DataType::UInt64,
            "Instruction address for this location (0 if unavailable); within its mapping's \
             memory range.",
        ),
        commented(
            "mapping_id",
            DataType::UInt64,
            "Id of the pprof.mappings row this location belongs to (0 if unknown).",
        ),
        commented(
            "lines",
            ab::list_lineref_type(),
            "The line table: a LIST of STRUCT(function_id UBIGINT, line BIGINT), innermost inlined \
             frame first. function_id joins to pprof.functions.function_id. Empty for an \
             unsymbolized location.",
        ),
    ])
}

impl TableFunction for Locations {
    fn name(&self) -> &str {
        "locations"
    }

    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "Locations Table",
            "Decode a pprof profile's location table: one row per location with `location_id` \
             (verbatim), `address`, `mapping_id`, and `lines` (LIST(STRUCT(function_id, line)), \
             innermost inlined frame first). Join `mapping_id` to pprof.mappings and \
             `lines[].function_id` to pprof.functions to symbolize. `src` may be a path, glob, \
             list, or BLOB; a bad file yields one error row.",
            "pprof locations: `location_id`, `address`, `mapping_id`, and `lines` \
             (STRUCT(function_id, line)[]). Join `mapping_id` to `mappings`, `lines[].function_id` \
             to `functions`.",
            "pprof, locations, location table, address, mapping id, line table, inlined frames, \
             function id, symbolization, join",
        );
        tags.push((
            "vgi.result_columns_md".into(),
            "| column | type | description |\n\
             |---|---|---|\n\
             | `location_id` | UBIGINT | Location id (verbatim). |\n\
             | `address` | UBIGINT | Instruction address. |\n\
             | `mapping_id` | UBIGINT | Owning mapping id. |\n\
             | `lines` | STRUCT(function_id UBIGINT, line BIGINT)[] | Line table, innermost first. |\n\
             | `file` | VARCHAR | Source path (NULL for BLOB input). |\n\
             | `error` | VARCHAR | NULL on success, else the decode error. |"
                .into(),
        ));
        tags.push(("vgi.executable_examples".into(), EXECUTABLE_EXAMPLES.into()));
        FunctionMetadata {
            description: "Decode a pprof profile's location table".into(),
            examples: vec![FunctionExample {
                sql: "SELECT location_id, address, mapping_id, lines FROM \
                      pprof.main.locations('cpu.pb.gz') WHERE error IS NULL ORDER BY location_id;"
                    .into(),
                description: "Locations with their line table (inlined frames).".into(),
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
    let mut location_id: Vec<Option<u64>> = Vec::new();
    let mut address: Vec<Option<u64>> = Vec::new();
    let mut mapping_id: Vec<Option<u64>> = Vec::new();
    let mut lines: Vec<Option<Vec<LineRef>>> = Vec::new();
    let mut file: Vec<Option<String>> = Vec::new();
    let mut error: Vec<Option<String>> = Vec::new();

    for unit in units {
        match &unit.result {
            Ok(decoded) => {
                for l in decoded.locations() {
                    location_id.push(Some(l.location_id));
                    address.push(Some(l.address));
                    mapping_id.push(Some(l.mapping_id));
                    lines.push(Some(l.lines));
                    file.push(unit.file.clone());
                    error.push(None);
                }
            }
            Err(msg) => {
                location_id.push(None);
                address.push(None);
                mapping_id.push(None);
                lines.push(None);
                file.push(unit.file.clone());
                error.push(Some(msg.clone()));
            }
        }
    }

    RecordBatch::try_new(
        schema.clone(),
        vec![
            ab::u64_col(&location_id),
            ab::u64_col(&address),
            ab::u64_col(&mapping_id),
            ab::list_lineref_col(&lines),
            ab::str_col(&file),
            ab::str_col(&error),
        ],
    )
    .map_err(|e| RpcError::runtime_error(e.to_string()))
}
