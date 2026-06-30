//! Arrow type definitions and column builders shared by the pprof table
//! functions.
//!
//! Each nested column's Arrow type is defined **once** here and reused by both a
//! table's `output_schema` (returned from `on_bind`) and its producer (which
//! builds the arrays), so the declared schema and the built `RecordBatch` always
//! agree exactly — a mismatch makes `RecordBatch::try_new` fail at runtime.
//!
//! The builders are columnar: a table collects one `Option<T>` per row (a `None`
//! marks a per-file **error row**, where every data column is NULL), then calls
//! the matching builder to turn that column into an `ArrayRef`.

use std::sync::Arc;

use arrow_array::builder::{Int64Builder, StringBuilder, UInt64Builder};
use arrow_array::{ArrayRef, ListArray, MapArray, StructArray};
use arrow_buffer::{NullBuffer, OffsetBuffer, ScalarBuffer};
use arrow_schema::{DataType, Field, FieldRef, Fields};

use pprof_core::{Frame, LineRef, ValueType};

/// The `item` field wrapping a list's element type (Arrow/DuckDB convention).
pub fn list_field(inner: DataType) -> FieldRef {
    Arc::new(Field::new("item", inner, true))
}

/// `STRUCT(type VARCHAR, unit VARCHAR)` — a resolved `ValueType`.
pub fn value_type_fields() -> Fields {
    Fields::from(vec![
        Field::new("type", DataType::Utf8, true),
        Field::new("unit", DataType::Utf8, true),
    ])
}

/// `STRUCT(function VARCHAR, filename VARCHAR, line BIGINT, address UBIGINT)` —
/// one pre-resolved stack frame.
pub fn frame_fields() -> Fields {
    Fields::from(vec![
        Field::new("function", DataType::Utf8, true),
        Field::new("filename", DataType::Utf8, true),
        Field::new("line", DataType::Int64, true),
        Field::new("address", DataType::UInt64, true),
    ])
}

/// `STRUCT(function_id UBIGINT, line BIGINT)` — a location's line-table entry.
pub fn lineref_fields() -> Fields {
    Fields::from(vec![
        Field::new("function_id", DataType::UInt64, true),
        Field::new("line", DataType::Int64, true),
    ])
}

/// The single child field of a `MAP(VARCHAR, VARCHAR)`: a non-null
/// `STRUCT(key, value)` named `entries`, key non-null, value nullable.
pub fn map_entries_field() -> FieldRef {
    let entries = Fields::from(vec![
        Field::new("key", DataType::Utf8, false),
        Field::new("value", DataType::Utf8, true),
    ]);
    Arc::new(Field::new("entries", DataType::Struct(entries), false))
}

/// The `MAP(VARCHAR, VARCHAR)` data type (labels column).
pub fn map_type() -> DataType {
    DataType::Map(map_entries_field(), false)
}

/// `LIST(BIGINT)` data type.
pub fn list_i64_type() -> DataType {
    DataType::List(list_field(DataType::Int64))
}

/// `LIST(UBIGINT)` data type.
pub fn list_u64_type() -> DataType {
    DataType::List(list_field(DataType::UInt64))
}

/// `LIST(STRUCT(function, filename, line, address))` data type.
pub fn list_frame_type() -> DataType {
    DataType::List(list_field(DataType::Struct(frame_fields())))
}

/// `LIST(STRUCT(function_id, line))` data type.
pub fn list_lineref_type() -> DataType {
    DataType::List(list_field(DataType::Struct(lineref_fields())))
}

/// `LIST(STRUCT(type, unit))` data type.
pub fn list_value_type_type() -> DataType {
    DataType::List(list_field(DataType::Struct(value_type_fields())))
}

// ---- column builders -------------------------------------------------------

/// Build a nullable `BIGINT` column.
pub fn i64_col(rows: &[Option<i64>]) -> ArrayRef {
    let mut b = Int64Builder::with_capacity(rows.len());
    for r in rows {
        b.append_option(*r);
    }
    Arc::new(b.finish())
}

/// Build a nullable `UBIGINT` column.
pub fn u64_col(rows: &[Option<u64>]) -> ArrayRef {
    let mut b = UInt64Builder::with_capacity(rows.len());
    for r in rows {
        b.append_option(*r);
    }
    Arc::new(b.finish())
}

/// Build a nullable `VARCHAR` column.
pub fn str_col(rows: &[Option<String>]) -> ArrayRef {
    let mut b = StringBuilder::new();
    for r in rows {
        b.append_option(r.as_deref());
    }
    Arc::new(b.finish())
}

/// Offsets + null buffer for a list/map column from per-row element counts.
fn offsets_and_nulls(counts: &[Option<usize>]) -> (OffsetBuffer<i32>, NullBuffer) {
    let mut offsets: Vec<i32> = Vec::with_capacity(counts.len() + 1);
    let mut valid: Vec<bool> = Vec::with_capacity(counts.len());
    offsets.push(0);
    let mut total: i32 = 0;
    for c in counts {
        if let Some(n) = c {
            total += *n as i32;
            valid.push(true);
        } else {
            valid.push(false);
        }
        offsets.push(total);
    }
    (
        OffsetBuffer::new(ScalarBuffer::from(offsets)),
        NullBuffer::from(valid),
    )
}

/// Build a `LIST(BIGINT)` column; a `None` row is a NULL list.
pub fn list_i64_col(rows: &[Option<Vec<i64>>]) -> ArrayRef {
    let counts: Vec<Option<usize>> = rows.iter().map(|r| r.as_ref().map(Vec::len)).collect();
    let (offsets, nulls) = offsets_and_nulls(&counts);
    let mut b = Int64Builder::new();
    for r in rows.iter().flatten() {
        for v in r {
            b.append_value(*v);
        }
    }
    Arc::new(ListArray::new(
        list_field(DataType::Int64),
        offsets,
        Arc::new(b.finish()),
        Some(nulls),
    ))
}

/// Build a `LIST(UBIGINT)` column; a `None` row is a NULL list.
pub fn list_u64_col(rows: &[Option<Vec<u64>>]) -> ArrayRef {
    let counts: Vec<Option<usize>> = rows.iter().map(|r| r.as_ref().map(Vec::len)).collect();
    let (offsets, nulls) = offsets_and_nulls(&counts);
    let mut b = UInt64Builder::new();
    for r in rows.iter().flatten() {
        for v in r {
            b.append_value(*v);
        }
    }
    Arc::new(ListArray::new(
        list_field(DataType::UInt64),
        offsets,
        Arc::new(b.finish()),
        Some(nulls),
    ))
}

/// Build a `MAP(VARCHAR, VARCHAR)` column from `(key, value)` pairs per row; a
/// `None` row is a NULL map. Keys are assumed already de-duplicated by the core.
pub fn map_col(rows: &[Option<Vec<(String, String)>>]) -> ArrayRef {
    let counts: Vec<Option<usize>> = rows.iter().map(|r| r.as_ref().map(Vec::len)).collect();
    let (offsets, nulls) = offsets_and_nulls(&counts);
    let mut keys = StringBuilder::new();
    let mut vals = StringBuilder::new();
    for pairs in rows.iter().flatten() {
        for (k, v) in pairs {
            keys.append_value(k);
            vals.append_value(v);
        }
    }
    let DataType::Struct(entry_fields) = map_entries_field().data_type().clone() else {
        unreachable!("map entries field is a struct")
    };
    let entries = StructArray::new(
        entry_fields,
        vec![Arc::new(keys.finish()), Arc::new(vals.finish())],
        None,
    );
    Arc::new(MapArray::new(
        map_entries_field(),
        offsets,
        entries,
        Some(nulls),
        false,
    ))
}

/// Build a `STRUCT(type, unit)` column; a `None` row is a NULL struct.
pub fn value_type_struct_col(rows: &[Option<ValueType>]) -> ArrayRef {
    let mut ty = StringBuilder::new();
    let mut unit = StringBuilder::new();
    let mut valid = Vec::with_capacity(rows.len());
    for r in rows {
        match r {
            Some(vt) => {
                ty.append_value(&vt.r#type);
                unit.append_value(&vt.unit);
                valid.push(true);
            }
            None => {
                ty.append_null();
                unit.append_null();
                valid.push(false);
            }
        }
    }
    Arc::new(StructArray::new(
        value_type_fields(),
        vec![Arc::new(ty.finish()), Arc::new(unit.finish())],
        Some(NullBuffer::from(valid)),
    ))
}

/// Build a `LIST(STRUCT(type, unit))` column; a `None` row is a NULL list.
pub fn list_value_type_col(rows: &[Option<Vec<ValueType>>]) -> ArrayRef {
    let counts: Vec<Option<usize>> = rows.iter().map(|r| r.as_ref().map(Vec::len)).collect();
    let (offsets, nulls) = offsets_and_nulls(&counts);
    let mut ty = StringBuilder::new();
    let mut unit = StringBuilder::new();
    for vt in rows.iter().flatten().flatten() {
        ty.append_value(&vt.r#type);
        unit.append_value(&vt.unit);
    }
    let child = StructArray::new(
        value_type_fields(),
        vec![Arc::new(ty.finish()), Arc::new(unit.finish())],
        None,
    );
    Arc::new(ListArray::new(
        list_field(DataType::Struct(value_type_fields())),
        offsets,
        Arc::new(child),
        Some(nulls),
    ))
}

/// Build a `LIST(STRUCT(function, filename, line, address))` column; a `None`
/// row is a NULL list.
pub fn list_frame_col(rows: &[Option<Vec<Frame>>]) -> ArrayRef {
    let counts: Vec<Option<usize>> = rows.iter().map(|r| r.as_ref().map(Vec::len)).collect();
    let (offsets, nulls) = offsets_and_nulls(&counts);
    let mut function = StringBuilder::new();
    let mut filename = StringBuilder::new();
    let mut line = Int64Builder::new();
    let mut address = UInt64Builder::new();
    for fr in rows.iter().flatten().flatten() {
        function.append_option(fr.function.as_deref());
        filename.append_option(fr.filename.as_deref());
        line.append_option(fr.line);
        address.append_value(fr.address);
    }
    let child = StructArray::new(
        frame_fields(),
        vec![
            Arc::new(function.finish()),
            Arc::new(filename.finish()),
            Arc::new(line.finish()),
            Arc::new(address.finish()),
        ],
        None,
    );
    Arc::new(ListArray::new(
        list_field(DataType::Struct(frame_fields())),
        offsets,
        Arc::new(child),
        Some(nulls),
    ))
}

/// Build a `LIST(STRUCT(function_id, line))` column; a `None` row is a NULL list.
pub fn list_lineref_col(rows: &[Option<Vec<LineRef>>]) -> ArrayRef {
    let counts: Vec<Option<usize>> = rows.iter().map(|r| r.as_ref().map(Vec::len)).collect();
    let (offsets, nulls) = offsets_and_nulls(&counts);
    let mut function_id = UInt64Builder::new();
    let mut line = Int64Builder::new();
    for lr in rows.iter().flatten().flatten() {
        function_id.append_value(lr.function_id);
        line.append_value(lr.line);
    }
    let child = StructArray::new(
        lineref_fields(),
        vec![Arc::new(function_id.finish()), Arc::new(line.finish())],
        None,
    );
    Arc::new(ListArray::new(
        list_field(DataType::Struct(lineref_fields())),
        offsets,
        Arc::new(child),
        Some(nulls),
    ))
}
