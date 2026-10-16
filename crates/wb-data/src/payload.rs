//! MCAP payloads that are not CDR, decoded into rows: protobuf, with the
//! `FileDescriptorSet` its schema holds (`prost-reflect`), and JSON. Rows become columns
//! through Arrow's JSON decoder, as in `rows.rs`; byte fields become their length, so
//! images and point clouds keep their size only.

use std::sync::Arc;

use datafusion::arrow::array::StructArray;
use datafusion::arrow::datatypes::{DataType, Schema};
use datafusion::arrow::json::reader::{Decoder, ReaderBuilder, infer_json_schema_from_iterator};
use prost_reflect::{DescriptorPool, DynamicMessage, Kind, MessageDescriptor, SerializeOptions};
use serde_json::Value;

use crate::extract::BATCH;

/// Protobuf as JSON: proto field names, 64-bit integers as numbers, default values kept
/// so that every row has every field.
const OPTIONS: SerializeOptions = SerializeOptions::new()
    .stringify_64_bit_integers(false)
    .use_proto_field_name(true)
    .skip_default_fields(false);

/// The decoded fields of one topic's messages, a batch at a time.
pub(crate) struct JsonRows {
    /// The message type of a protobuf topic; `None` for JSON.
    proto: Option<MessageDescriptor>,
    rows: Vec<Value>,
    /// Built from the first batch's columns.
    decoder: Option<Decoder>,
    pub failures: u64,
    pub first_error: Option<String>,
}

impl JsonRows {
    /// A protobuf topic of type `type_name`, from its schema's `FileDescriptorSet`.
    pub fn protobuf(type_name: &str, schema: &[u8]) -> Result<Self, String> {
        let pool = DescriptorPool::decode(schema).map_err(|e| e.to_string())?;
        let proto = pool
            .get_message_by_name(type_name)
            .ok_or_else(|| format!("the schema does not define {type_name}"))?;
        Ok(Self::new(Some(proto)))
    }

    pub fn json() -> Self {
        Self::new(None)
    }

    fn new(proto: Option<MessageDescriptor>) -> Self {
        Self {
            proto,
            rows: Vec::with_capacity(BATCH),
            decoder: None,
            failures: 0,
            first_error: None,
        }
    }

    fn fail(&mut self, e: impl std::fmt::Display) -> Result<bool, String> {
        self.failures += 1;
        self.first_error.get_or_insert_with(|| e.to_string());
        Ok(false)
    }

    /// Adds one message; `false` (and a failure counted) when it does not decode.
    pub fn push(&mut self, payload: &[u8]) -> Result<bool, String> {
        let row = match &self.proto {
            Some(desc) => match DynamicMessage::decode(desc.clone(), payload) {
                Ok(m) => match m.serialize_with_options(serde_json::value::Serializer, &OPTIONS) {
                    Ok(mut v) => {
                        bytes_to_len(desc, &mut v);
                        v
                    }
                    Err(e) => return self.fail(e),
                },
                Err(e) => return self.fail(e),
            },
            None => match serde_json::from_slice::<Value>(payload) {
                Ok(v @ Value::Object(_)) => v,
                // A JSON value that is not an object is one column, `value`.
                Ok(v) => serde_json::json!({ "value": v }),
                Err(e) => return self.fail(e),
            },
        };
        self.rows.push(row);
        Ok(true)
    }

    /// The fields of the messages pushed since the last call, one struct row each.
    pub fn take(&mut self) -> Result<StructArray, String> {
        let rows = std::mem::take(&mut self.rows);
        if self.decoder.is_none() {
            let inferred =
                infer_json_schema_from_iterator(rows.iter().map(Ok)).map_err(|e| e.to_string())?;
            // Fields that were always null have no type to store.
            let fields: Vec<_> = inferred
                .fields()
                .iter()
                .filter(|f| *f.data_type() != DataType::Null)
                .cloned()
                .collect();
            self.decoder = Some(
                ReaderBuilder::new(Arc::new(Schema::new(fields)))
                    .with_batch_size(BATCH)
                    .build_decoder()
                    .map_err(|e| e.to_string())?,
            );
        }
        let decoder = self.decoder.as_mut().expect("built above");
        decoder.serialize(&rows).map_err(|e| e.to_string())?;
        match decoder.flush().map_err(|e| e.to_string())? {
            Some(batch) => Ok(batch.into()),
            None => Ok(StructArray::new_empty_fields(0, None)),
        }
    }
}

/// Replaces the base64 text of `bytes` fields (protobuf JSON) with the number of bytes.
fn bytes_to_len(desc: &MessageDescriptor, v: &mut Value) {
    let Value::Object(map) = v else {
        return;
    };
    for field in desc.fields() {
        let Some(value) = map.get_mut(field.name()) else {
            continue;
        };
        match field.kind() {
            Kind::Bytes => match value {
                Value::String(s) => *value = base64_len(s).into(),
                Value::Array(items) => {
                    for item in items {
                        if let Value::String(s) = item {
                            *item = base64_len(s).into();
                        }
                    }
                }
                _ => {}
            },
            Kind::Message(inner) if !field.is_map() => match value {
                Value::Array(items) => {
                    for item in items {
                        bytes_to_len(&inner, item);
                    }
                }
                other => bytes_to_len(&inner, other),
            },
            _ => {}
        }
    }
}

/// The number of bytes that base64 text decodes to.
fn base64_len(s: &str) -> u64 {
    let padding = s.bytes().rev().take_while(|&b| b == b'=').count();
    ((s.len() / 4) * 3).saturating_sub(padding) as u64
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use datafusion::arrow::array::{Array, AsArray, RecordBatch};
    use datafusion::arrow::datatypes::{Float64Type, Int64Type};
    use prost::Message as _;
    use prost_reflect::Value as Pv;
    use prost_types::field_descriptor_proto::{Label, Type};
    use prost_types::{
        DescriptorProto, EnumDescriptorProto, EnumValueDescriptorProto, FieldDescriptorProto,
        FileDescriptorProto, FileDescriptorSet,
    };
    use wb_core::roots::RecordingKind;

    use super::*;

    fn field(
        name: &str,
        n: i32,
        t: Type,
        label: Label,
        type_name: Option<&str>,
    ) -> FieldDescriptorProto {
        FieldDescriptorProto {
            name: Some(name.into()),
            number: Some(n),
            r#type: Some(t as i32),
            label: Some(label as i32),
            type_name: type_name.map(Into::into),
            ..Default::default()
        }
    }

    /// `demo.Reading`: scalars, a list, bytes, a nested message and an enum.
    fn descriptors() -> Vec<u8> {
        let inner = DescriptorProto {
            name: Some("Inner".into()),
            field: vec![field("label", 1, Type::String, Label::Optional, None)],
            ..Default::default()
        };
        let reading = DescriptorProto {
            name: Some("Reading".into()),
            field: vec![
                field("temperature", 1, Type::Double, Label::Optional, None),
                field("count", 2, Type::Int64, Label::Optional, None),
                field("values", 3, Type::Float, Label::Repeated, None),
                field("image", 4, Type::Bytes, Label::Optional, None),
                field(
                    "inner",
                    5,
                    Type::Message,
                    Label::Optional,
                    Some(".demo.Inner"),
                ),
                field("mode", 6, Type::Enum, Label::Optional, Some(".demo.Mode")),
            ],
            ..Default::default()
        };
        let mode = EnumDescriptorProto {
            name: Some("Mode".into()),
            value: ["IDLE", "RUN"]
                .iter()
                .enumerate()
                .map(|(i, n)| EnumValueDescriptorProto {
                    name: Some((*n).into()),
                    number: Some(i as i32),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("demo.proto".into()),
                package: Some("demo".into()),
                syntax: Some("proto3".into()),
                message_type: vec![inner, reading],
                enum_type: vec![mode],
                ..Default::default()
            }],
        }
        .encode_to_vec()
    }

    fn stream(out: &Path, x: &crate::extract::Extraction, name: &str) -> RecordBatch {
        let meta = x.streams.iter().find(|s| s.name == name).unwrap();
        let file = std::fs::File::open(out.join(&meta.dataset)).unwrap();
        let reader =
            parquet::arrow::arrow_reader::ParquetRecordBatchReader::try_new(file, 1 << 20).unwrap();
        let batches: Vec<RecordBatch> = reader.map(|b| b.unwrap()).collect();
        datafusion::arrow::compute::concat_batches(&batches[0].schema(), &batches).unwrap()
    }

    #[tokio::test]
    async fn protobuf_and_json_channels_of_an_mcap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rec.mcap");
        let set = descriptors();
        let pool = DescriptorPool::decode(set.as_slice()).unwrap();
        let (reading, inner) = (
            pool.get_message_by_name("demo.Reading").unwrap(),
            pool.get_message_by_name("demo.Inner").unwrap(),
        );
        let mut w = mcap::Writer::new(std::io::BufWriter::new(
            std::fs::File::create(&path).unwrap(),
        ))
        .unwrap();
        let schema = w.add_schema("demo.Reading", "protobuf", &set).unwrap();
        let proto = w
            .add_channel(schema, "/reading", "protobuf", &BTreeMap::new())
            .unwrap();
        let schema = w.add_schema("Status", "jsonschema", b"{}").unwrap();
        let json = w
            .add_channel(schema, "/status", "json", &BTreeMap::new())
            .unwrap();
        for i in 0..3u32 {
            let mut m = DynamicMessage::new(reading.clone());
            m.set_field_by_name("temperature", Pv::F64(20.5 + f64::from(i)));
            m.set_field_by_name("count", Pv::I64(9_007_199_254_740_993 + i64::from(i)));
            m.set_field_by_name("values", Pv::List(vec![Pv::F32(1.5), Pv::F32(-2.25)]));
            m.set_field_by_name("image", Pv::Bytes(vec![7u8; 1000].into()));
            let mut label = DynamicMessage::new(inner.clone());
            label.set_field_by_name("label", Pv::String(format!("n{i}")));
            m.set_field_by_name("inner", Pv::Message(label));
            m.set_field_by_name("mode", Pv::EnumNumber(1));
            let header = |channel_id, log_time| mcap::records::MessageHeader {
                channel_id,
                sequence: i,
                log_time,
                publish_time: 0,
            };
            w.write_to_known_channel(
                &header(proto, 1_000_000_000 + u64::from(i) * 1000),
                &m.encode_to_vec(),
            )
            .unwrap();
            let status = serde_json::json!({"ok": i % 2 == 0, "level": 2.5 * f64::from(i), "tags": ["a", "b"]});
            w.write_to_known_channel(
                &header(json, 1_000_000_500 + u64::from(i) * 1000),
                status.to_string().as_bytes(),
            )
            .unwrap();
        }
        w.write_to_known_channel(
            &mcap::records::MessageHeader {
                channel_id: proto,
                sequence: 9,
                log_time: 1_000_009_000,
                publish_time: 0,
            },
            b"\xff\xff not protobuf",
        )
        .unwrap();
        w.finish().unwrap();

        let out = dir.path().join("out");
        let x = crate::extract::extract(&path, RecordingKind::Mcap, None, &out)
            .await
            .unwrap();
        // Values as python's mcap-protobuf-support reads them (enums by name here).
        let r = stream(&out, &x, "/reading");
        assert_eq!(r.num_rows(), 3);
        let f64s = |b: &RecordBatch, c: &str| {
            b.column_by_name(c)
                .unwrap()
                .as_primitive::<Float64Type>()
                .values()
                .to_vec()
        };
        let i64s = |b: &RecordBatch, c: &str| {
            b.column_by_name(c)
                .unwrap()
                .as_primitive::<Int64Type>()
                .values()
                .to_vec()
        };
        assert_eq!(f64s(&r, "temperature"), [20.5, 21.5, 22.5]);
        assert_eq!(
            i64s(&r, "count"),
            [
                9_007_199_254_740_993,
                9_007_199_254_740_994,
                9_007_199_254_740_995
            ]
        );
        assert_eq!(i64s(&r, "image"), [1000, 1000, 1000]);
        let values = r
            .column_by_name("values")
            .unwrap()
            .as_list::<i32>()
            .value(0);
        assert_eq!(values.as_primitive::<Float64Type>().values(), &[1.5, -2.25]);
        let inner = r.column_by_name("inner").unwrap().as_struct();
        assert_eq!(
            inner
                .column_by_name("label")
                .unwrap()
                .as_string::<i32>()
                .value(2),
            "n2"
        );
        assert_eq!(
            r.column_by_name("mode")
                .unwrap()
                .as_string::<i32>()
                .value(0),
            "RUN"
        );
        let meta = x.streams.iter().find(|s| s.name == "/reading").unwrap();
        assert!(meta.decoded);
        assert!(
            meta.notes[0].starts_with("1 message(s) could not be decoded"),
            "{:?}",
            meta.notes
        );

        let s = stream(&out, &x, "/status");
        assert_eq!(f64s(&s, "level"), [0.0, 2.5, 5.0]);
        let ok = s.column_by_name("ok").unwrap().as_boolean();
        assert_eq!(
            (ok.value(0), ok.value(1), ok.is_null(2)),
            (true, false, false)
        );
    }
}
