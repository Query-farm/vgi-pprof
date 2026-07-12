//! `pprof.functions(src)` — the function table: function_id → name / system_name
//! / filename / start_line (ids passed through verbatim for joins).

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
    "description": "List the functions in a profile.",
    "sql": "SELECT function_id, name, filename, start_line FROM pprof.main.functions('data/go_cpu.pb.gz') WHERE error IS NULL ORDER BY function_id LIMIT 10"
  },
  {
    "description": "Which source files contribute the most symbols (functions) to a profile.",
    "sql": "SELECT filename, count(*) AS fns FROM pprof.main.functions('data/go_cpu.pb.gz') WHERE error IS NULL GROUP BY filename ORDER BY fns DESC, filename LIMIT 5"
  }
]"#;

pub struct Functions;

pub fn output_schema() -> SchemaRef {
    schema_with_trailing(vec![
        commented(
            "function_id",
            DataType::UInt64,
            "The function's unique id within the profile (emitted verbatim); referenced by \
             pprof.locations.lines[].function_id.",
        ),
        commented(
            "name",
            DataType::Utf8,
            "Human-readable function name, or NULL if unset.",
        ),
        commented(
            "system_name",
            DataType::Utf8,
            "System/mangled function name (e.g. the C++ mangled symbol), or NULL if unset.",
        ),
        commented(
            "filename",
            DataType::Utf8,
            "Source file that contains the function, or NULL if unset.",
        ),
        commented(
            "start_line",
            DataType::Int64,
            "First source line of the function (0 if unknown).",
        ),
    ])
}

impl TableFunction for Functions {
    fn name(&self) -> &str {
        "functions"
    }

    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "Functions Table",
            "Decode a pprof profile's function table: one row per function with `function_id` \
             (verbatim), `name`, `system_name` (mangled), `filename`, and `start_line`. Join \
             `function_id` from pprof.locations.lines to symbolize raw locations. `src` may be a \
             path, glob, list, or BLOB; a bad file yields one error row.",
            "pprof functions: `function_id`, `name`, `system_name`, `filename`, `start_line`. Join \
             `function_id` to `locations.lines[].function_id`.",
            "pprof, functions, function table, symbol, name, mangled, system name, filename, \
             start line, symbolization, join",
            "Raw profile graph",
        );
        let mut cols = vec![
            (
                "function_id",
                "UBIGINT",
                "The function's unique id within the profile (verbatim); referenced by \
                 pprof.locations.lines[].function_id.",
            ),
            (
                "name",
                "VARCHAR",
                "Human-readable function name, or NULL if unset.",
            ),
            (
                "system_name",
                "VARCHAR",
                "System/mangled function name (e.g. the C++ mangled symbol), or NULL if unset.",
            ),
            (
                "filename",
                "VARCHAR",
                "Source file that contains the function, or NULL if unset.",
            ),
            (
                "start_line",
                "BIGINT",
                "First source line of the function (0 if unknown).",
            ),
        ];
        cols.extend(crate::meta::trailing_result_columns());
        tags.push((
            "vgi.result_columns_schema".into(),
            crate::meta::result_columns_schema(&cols),
        ));
        tags.push(("vgi.executable_examples".into(), EXECUTABLE_EXAMPLES.into()));
        FunctionMetadata {
            description: "Decode a pprof profile's function table".into(),
            examples: vec![FunctionExample {
                sql: "SELECT function_id, name, filename, start_line FROM \
                      pprof.main.functions('data/go_cpu.pb.gz') WHERE error IS NULL ORDER BY function_id;"
                    .into(),
                description: "List the functions defined in a profile.".into(),
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
    let mut function_id: Vec<Option<u64>> = Vec::new();
    let mut name: Vec<Option<String>> = Vec::new();
    let mut system_name: Vec<Option<String>> = Vec::new();
    let mut filename: Vec<Option<String>> = Vec::new();
    let mut start_line: Vec<Option<i64>> = Vec::new();
    let mut file: Vec<Option<String>> = Vec::new();
    let mut error: Vec<Option<String>> = Vec::new();

    for unit in units {
        match &unit.result {
            Ok(decoded) => {
                for f in decoded.functions() {
                    function_id.push(Some(f.function_id));
                    name.push(f.name);
                    system_name.push(f.system_name);
                    filename.push(f.filename);
                    start_line.push(Some(f.start_line));
                    file.push(unit.file.clone());
                    error.push(None);
                }
            }
            Err(msg) => {
                function_id.push(None);
                name.push(None);
                system_name.push(None);
                filename.push(None);
                start_line.push(None);
                file.push(unit.file.clone());
                error.push(Some(msg.clone()));
            }
        }
    }

    RecordBatch::try_new(
        schema.clone(),
        vec![
            ab::u64_col(&function_id),
            ab::str_col(&name),
            ab::str_col(&system_name),
            ab::str_col(&filename),
            ab::i64_col(&start_line),
            ab::str_col(&file),
            ab::str_col(&error),
        ],
    )
    .map_err(|e| RpcError::runtime_error(e.to_string()))
}
