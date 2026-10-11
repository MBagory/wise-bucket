//! Read-only SQL over the store, run by DataFusion.
//!
//! The session sees only what is registered here (`recordings`, `streams` and the
//! `stream()` table function); DDL, writes and other statements are refused, and
//! memory, time and result size are capped. Nothing outside the store is readable.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use datafusion::arrow::array::{
    ArrayRef, BooleanArray, Int32Array, Int64Array, RecordBatch, StringArray,
    TimestampNanosecondArray,
};
use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use datafusion::catalog::{TableFunctionImpl, TableProvider};
use datafusion::common::{DataFusionError, ScalarValue, plan_err};
use datafusion::datasource::MemTable;
use datafusion::execution::disk_manager::{DiskManagerBuilder, DiskManagerMode};
use datafusion::execution::runtime_env::RuntimeEnvBuilder;
use datafusion::functions::core::expr_fn::get_field;
use datafusion::logical_expr::{Expr, cast, ident, lit};
use datafusion::prelude::{DataFrame, SQLOptions, SessionConfig, SessionContext};
use futures::StreamExt;
use lance::datafusion::LanceTableProvider;
use serde::Serialize;
use wb_core::error::{ErrorKind, Result, WbError, err};

use crate::columns::{FlatColumn, flat_columns, time_type};
use crate::store::{Row, Store};

/// Bytes of rows returned at most (about 20k tokens): every row enters the agent's
/// context, and clients cut tool results beyond about 25k tokens.
pub const MAX_BYTES: usize = 60_000;
/// Longest text cell returned, in characters.
const MAX_CELL: usize = 1000;
/// Longest a query may run.
pub const TIMEOUT: Duration = Duration::from_secs(30);
/// Memory a query may use (no spilling to disk).
const MEMORY: usize = 1 << 30;

/// The result of a query.
#[derive(Debug, Clone, Serialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    /// One array per row, in the order of `columns`.
    pub rows: Vec<Vec<serde_json::Value>>,
    pub row_count: usize,
    pub truncated: bool,
    /// When truncated: why, and how to get the rest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

fn query_err(e: impl std::fmt::Display) -> WbError {
    err(ErrorKind::QueryFailed, e.to_string())
}

enum Cut {
    Limit(usize),
    Size,
}

/// Runs one read-only query over the store, returning at most `limit` rows and
/// [`MAX_BYTES`] of them.
pub async fn query(store: &Store, sql: &str, limit: Option<usize>) -> Result<QueryResult> {
    let ctx = context(store).map_err(query_err)?;
    let options = SQLOptions::new()
        .with_allow_ddl(false)
        .with_allow_dml(false)
        .with_allow_statements(false);
    let run = async {
        let mut df = ctx
            .sql_with_options(sql, options)
            .await
            .map_err(query_err)?;
        if let Some(n) = limit {
            // One more row tells whether more exist.
            df = df.limit(0, Some(n + 1)).map_err(query_err)?;
        }
        let columns: Vec<String> = df
            .schema()
            .fields()
            .iter()
            .map(|f| f.name().clone())
            .collect();
        let mut stream = df.execute_stream().await.map_err(query_err)?;
        let mut rows = Vec::new();
        let mut bytes = 0;
        while let Some(batch) = stream.next().await {
            for mut obj in to_json(&[batch.map_err(query_err)?])? {
                if let Some(n) = limit.filter(|&n| rows.len() == n) {
                    return Ok((columns, rows, Some(Cut::Limit(n))));
                }
                shorten(&mut obj);
                let row: Vec<serde_json::Value> = columns
                    .iter()
                    .map(|c| obj.get_mut(c.as_str()).map(serde_json::Value::take))
                    .map(Option::unwrap_or_default)
                    .collect();
                bytes += serde_json::to_vec(&row).map_err(query_err)?.len() + 1;
                if bytes > MAX_BYTES {
                    return Ok((columns, rows, Some(Cut::Size)));
                }
                rows.push(row);
            }
        }
        Ok::<_, WbError>((columns, rows, None))
    };
    let (columns, rows, cut) = tokio::time::timeout(TIMEOUT, run).await.map_err(|_| {
        query_err(format!(
            "the query ran for more than {} s and was stopped",
            TIMEOUT.as_secs()
        ))
    })??;
    let hint = cut.map(|cut| {
        // The next rows start after the last `t` returned, when the result has one.
        let next = columns
            .iter()
            .position(|c| c == "t")
            .and_then(|i| rows.last()?[i].as_str().map(str::to_string))
            .map_or("a time window on t (ORDER BY t)".to_string(), |last| {
                format!("WHERE t > TIMESTAMP '{last}' ORDER BY t")
            });
        match cut {
            Cut::Limit(n) => format!(
                "More rows exist beyond limit {n}: raise limit, or fetch the next rows with {next}."
            ),
            Cut::Size => format!(
                "Cut at {} rows to stay under {} KB: every row enters your context. Aggregate \
                 instead (date_bin with min, avg and max per bin: the Plot recipe), filter with \
                 WHERE, select fewer columns, or fetch the next rows with {next}.",
                rows.len(),
                MAX_BYTES / 1000
            ),
        }
    });
    Ok(QueryResult {
        columns,
        row_count: rows.len(),
        rows,
        truncated: hint.is_some(),
        hint,
    })
}

fn to_json(batches: &[RecordBatch]) -> Result<Vec<serde_json::Value>> {
    use datafusion::arrow::json::{WriterBuilder, writer::JsonArray};
    let mut buf = Vec::new();
    {
        let mut w = WriterBuilder::new()
            .with_explicit_nulls(true)
            .build::<_, JsonArray>(&mut buf);
        for b in batches {
            w.write(b).map_err(query_err)?;
        }
        w.finish().map_err(query_err)?;
    }
    if buf.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_slice(&buf).map_err(query_err)
}

fn shorten(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::String(s) if s.chars().count() > MAX_CELL => {
            *s = format!("{}…", s.chars().take(MAX_CELL).collect::<String>());
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(shorten),
        serde_json::Value::Object(o) => o.values_mut().for_each(shorten),
        _ => {}
    }
}

/// A session over the store, locked down.
pub fn context(store: &Store) -> datafusion::common::Result<SessionContext> {
    let rows = store.rows();
    let runtime = RuntimeEnvBuilder::new()
        .with_memory_limit(MEMORY, 1.0)
        .with_disk_manager_builder(
            DiskManagerBuilder::default().with_mode(DiskManagerMode::Disabled),
        )
        .build_arc()?;
    let config = SessionConfig::new().with_information_schema(true);
    let ctx = SessionContext::new_with_config_rt(config, runtime);
    ctx.register_table("recordings", Arc::new(recordings_table(&rows)?))?;
    ctx.register_table("streams", Arc::new(streams_table(&rows)?))?;
    let mut index: HashMap<String, Vec<(String, PathBuf)>> = HashMap::new();
    for r in &rows {
        if let (Some(meta), Some(dir)) = (&r.meta, &r.dir) {
            for s in &meta.streams {
                index
                    .entry(s.name.clone())
                    .or_default()
                    .push((r.recording.clone(), dir.join(&s.dataset)));
            }
        }
    }
    ctx.register_udtf(
        "stream",
        Arc::new(StreamFunction {
            index: Arc::new(index),
        }),
    );
    Ok(ctx)
}

fn text(v: Vec<Option<String>>) -> ArrayRef {
    Arc::new(StringArray::from(v))
}

fn times(v: Vec<Option<i64>>) -> ArrayRef {
    Arc::new(TimestampNanosecondArray::from(v))
}

fn json_name(v: impl Serialize) -> Option<String> {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
}

fn table(
    fields: Vec<(&str, DataType)>,
    columns: Vec<ArrayRef>,
) -> datafusion::common::Result<MemTable> {
    let schema: SchemaRef = Arc::new(Schema::new(
        fields
            .into_iter()
            .map(|(n, t)| Field::new(n, t, true))
            .collect::<Vec<_>>(),
    ));
    let batch = RecordBatch::try_new(schema.clone(), columns)?;
    MemTable::try_new(schema, vec![vec![batch]])
}

fn recordings_table(rows: &[Row]) -> datafusion::common::Result<MemTable> {
    let ts = time_type();
    table(
        vec![
            ("recording", DataType::Utf8),
            ("root", DataType::Utf8),
            ("path", DataType::Utf8),
            ("kind", DataType::Utf8),
            ("hardware", DataType::Utf8),
            ("status", DataType::Utf8),
            ("error", DataType::Utf8),
            ("size_bytes", DataType::Int64),
            ("modified", ts.clone()),
            ("start", ts.clone()),
            ("end", ts),
            ("streams", DataType::Int32),
            ("notes", DataType::Utf8),
        ],
        vec![
            text(rows.iter().map(|r| Some(r.recording.clone())).collect()),
            text(rows.iter().map(|r| Some(r.root.clone())).collect()),
            text(
                rows.iter()
                    .map(|r| Some(r.path.display().to_string()))
                    .collect(),
            ),
            text(rows.iter().map(|r| json_name(r.kind)).collect()),
            text(rows.iter().map(|r| r.hardware.clone()).collect()),
            text(rows.iter().map(|r| json_name(r.status)).collect()),
            text(rows.iter().map(|r| r.error.clone()).collect()),
            Arc::new(Int64Array::from(
                rows.iter().map(|r| r.size_bytes as i64).collect::<Vec<_>>(),
            )),
            times(rows.iter().map(|r| Some(r.mtime_ns)).collect()),
            times(
                rows.iter()
                    .map(|r| r.meta.as_ref().and_then(|m| m.start_ns()))
                    .collect(),
            ),
            times(
                rows.iter()
                    .map(|r| r.meta.as_ref().and_then(|m| m.end_ns()))
                    .collect(),
            ),
            Arc::new(Int32Array::from(
                rows.iter()
                    .map(|r| r.meta.as_ref().map(|m| m.streams.len() as i32))
                    .collect::<Vec<_>>(),
            )),
            text(
                rows.iter()
                    .map(|r| {
                        r.meta
                            .as_ref()
                            .filter(|m| !m.notes.is_empty())
                            .map(|m| m.notes.join("; "))
                    })
                    .collect(),
            ),
        ],
    )
}

fn streams_table(rows: &[Row]) -> datafusion::common::Result<MemTable> {
    let streams: Vec<(&str, &crate::sink::StreamMeta)> = rows
        .iter()
        .filter_map(|r| r.meta.as_ref().map(|m| (r.recording.as_str(), m)))
        .flat_map(|(rec, m)| m.streams.iter().map(move |s| (rec, s)))
        .collect();
    let ts = time_type();
    table(
        vec![
            ("recording", DataType::Utf8),
            ("stream", DataType::Utf8),
            ("type", DataType::Utf8),
            ("encoding", DataType::Utf8),
            ("rows", DataType::Int64),
            ("start", ts.clone()),
            ("end", ts),
            ("decoded", DataType::Boolean),
            ("columns", DataType::Utf8),
            ("notes", DataType::Utf8),
        ],
        vec![
            text(streams.iter().map(|(r, _)| Some(r.to_string())).collect()),
            text(streams.iter().map(|(_, s)| Some(s.name.clone())).collect()),
            text(streams.iter().map(|(_, s)| Some(s.type_.clone())).collect()),
            text(
                streams
                    .iter()
                    .map(|(_, s)| Some(s.encoding.clone()))
                    .collect(),
            ),
            Arc::new(Int64Array::from(
                streams
                    .iter()
                    .map(|(_, s)| s.rows as i64)
                    .collect::<Vec<_>>(),
            )),
            times(streams.iter().map(|(_, s)| s.start_ns).collect()),
            times(streams.iter().map(|(_, s)| s.end_ns).collect()),
            Arc::new(BooleanArray::from(
                streams.iter().map(|(_, s)| s.decoded).collect::<Vec<_>>(),
            )),
            text(
                streams
                    .iter()
                    .map(|(_, s)| Some(s.columns.join(", ")))
                    .collect(),
            ),
            text(
                streams
                    .iter()
                    .map(|(_, s)| (!s.notes.is_empty()).then(|| s.notes.join("; ")))
                    .collect(),
            ),
        ],
    )
}

/// SQL `LIKE`: `%` matches any text, `_` one character.
fn like(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '_' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '%' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '%')
}

/// `stream('<name>' [, '<recording LIKE pattern>'])`.
#[derive(Debug)]
struct StreamFunction {
    /// Stream name → `(recording, dataset)` of every recording that has it.
    index: Arc<HashMap<String, Vec<(String, PathBuf)>>>,
}

fn string_arg(args: &[Expr], i: usize) -> datafusion::common::Result<Option<String>> {
    match args.get(i) {
        None => Ok(None),
        Some(Expr::Literal(ScalarValue::Utf8(Some(s)), _)) => Ok(Some(s.clone())),
        Some(other) => plan_err!("stream() takes text literals, got {other}"),
    }
}

impl TableFunctionImpl for StreamFunction {
    fn call(&self, args: &[Expr]) -> datafusion::common::Result<Arc<dyn TableProvider>> {
        let Some(name) = string_arg(args, 0)? else {
            return plan_err!("stream() needs a stream name: stream('/imu')");
        };
        let pattern = string_arg(args, 1)?;
        let members: Vec<(String, PathBuf)> = self
            .index
            .get(&name)
            .into_iter()
            .flatten()
            .filter(|(rec, _)| pattern.as_deref().is_none_or(|p| like(p, rec)))
            .cloned()
            .collect();
        if members.is_empty() {
            return plan_err!(
                "no extracted stream named {name:?}{}. List them with: SELECT recording, stream FROM streams",
                pattern
                    .map(|p| format!(" in recordings matching {p:?}"))
                    .unwrap_or_default()
            );
        }
        // Opening Lance datasets is async; planning is not.
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(union_of(&name, members))
        })
    }
}

/// The union of a stream's datasets, each with a `recording` column, over the union
/// of their columns (missing ones are null). Nested structs are flattened into dotted
/// columns (`linear_acceleration.z`).
async fn union_of(
    name: &str,
    members: Vec<(String, PathBuf)>,
) -> datafusion::common::Result<Arc<dyn TableProvider>> {
    let mut providers = Vec::new();
    let mut columns: Vec<FlatColumn> = Vec::new();
    for (rec, path) in members {
        let ds = crate::sink::open(&path)
            .await
            .map_err(|e| DataFusionError::External(Box::new(e)))?;
        let provider = Arc::new(LanceTableProvider::new(ds, false, false));
        let own = flat_columns(&provider.schema());
        for c in &own {
            match columns.iter().find(|x| x.name == c.name) {
                None => columns.push(c.clone()),
                Some(x) if x.data_type != c.data_type => {
                    return plan_err!(
                        "column {:?} of stream {name:?} has different types across recordings ({} and {}); restrict the recordings with stream({name:?}, '<recording LIKE pattern>')",
                        c.name,
                        x.data_type,
                        c.data_type
                    );
                }
                Some(_) => {}
            }
        }
        providers.push((rec, provider, own));
    }
    let ctx = SessionContext::new();
    let mut union: Option<DataFrame> = None;
    for (rec, provider, own) in providers {
        let mut exprs = vec![lit(rec).alias("recording")];
        for c in &columns {
            let e = match own.iter().find(|o| o.name == c.name) {
                Some(o) => {
                    let mut e = ident(&o.path[0]);
                    for p in &o.path[1..] {
                        e = get_field(e, p.as_str());
                    }
                    e
                }
                None => cast(lit(ScalarValue::Null), c.data_type.clone()),
            };
            exprs.push(e.alias(&c.name));
        }
        let part = ctx.read_table(provider)?.select(exprs)?;
        union = Some(match union {
            None => part,
            Some(u) => u.union(part)?,
        });
    }
    Ok(union.expect("at least one member").into_view())
}

#[cfg(test)]
mod tests {
    use super::like;

    #[test]
    fn like_patterns() {
        assert!(like("bags:%", "bags:run1/a.mcap"));
        assert!(like("%run_/%", "bags:run1/a.mcap"));
        assert!(!like("demo:%", "bags:run1"));
        assert!(like("%", ""));
        assert!(!like("a_", "a"));
    }
}
