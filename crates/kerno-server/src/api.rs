//! The requests and answers of the API, as the HTTP JSON API sends them. The gRPC side converts
//! its messages to and from these, so both transports run one core ([`crate::service`]).
//!
//! Ids are 32 lowercase hex digits. A unit's content is a [`Payload`]: `{"text": ...}`,
//! `{"hex": ...}` or, for a unit type that has a schema, `{"fields": {...}}`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use kerno::FieldValue;
use serde::de::Error as _;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

// ------------------------------------------------------------------ bytes

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Bytes from hex digits, either case.
pub fn from_hex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("expected an even number of hex digits".into());
    }
    Ok(text
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            // Both digits were checked above.
            let digit = |b: u8| char::from(b).to_digit(16).unwrap_or(0) as u8;
            digit(pair[0]) << 4 | digit(pair[1])
        })
        .collect())
}

/// Bytes that travel as a hex string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hex(pub Vec<u8>);

impl Serialize for Hex {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&to_hex(&self.0))
    }
}

impl<'de> Deserialize<'de> for Hex {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        from_hex(&String::deserialize(d)?).map(Hex).map_err(D::Error::custom)
    }
}

// ----------------------------------------------------------------- fields

/// The value of one field of a typed record: a JSON boolean, integer, float or string, or
/// `{"hex": ...}` for bytes. A JSON number without a fraction is an `Int`; the server turns it into
/// a `Float` where the schema says the field is one (JavaScript cannot write `5.0`).
#[derive(Clone, Debug, PartialEq)]
pub struct Field(pub FieldValue);

impl Serialize for Field {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match &self.0 {
            FieldValue::Bool(b) => s.serialize_bool(*b),
            FieldValue::Int(i) => s.serialize_i64(*i),
            // ponytail: JSON has no infinity, so serde_json writes `null`; a record written through
            // the Rust API with one reads back as a missing value here.
            FieldValue::Float(f) => s.serialize_f64(*f),
            FieldValue::Text(t) => s.serialize_str(t),
            FieldValue::Bytes(b) => {
                let mut map = s.serialize_map(Some(1))?;
                map.serialize_entry("hex", &to_hex(b))?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Field {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = match Value::deserialize(d)? {
            Value::Bool(b) => FieldValue::Bool(b),
            Value::Number(n) => match n.as_i64() {
                Some(i) => FieldValue::Int(i),
                None => FieldValue::Float(n.as_f64().ok_or_else(|| D::Error::custom("number out of range"))?),
            },
            Value::String(s) => FieldValue::Text(s),
            Value::Object(m) if m.len() == 1 && m.contains_key("hex") => {
                let digits = m["hex"].as_str().ok_or_else(|| D::Error::custom("\"hex\" must be a string"))?;
                FieldValue::Bytes(from_hex(digits).map_err(D::Error::custom)?)
            }
            _ => {
                return Err(D::Error::custom("a field value is a boolean, a number, a string or {\"hex\": \"...\"}"));
            }
        };
        Ok(Field(value))
    }
}

// ---------------------------------------------------------------- payload

/// The content of a unit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Payload {
    Text(String),
    Hex(Hex),
    Fields(BTreeMap<String, Field>),
}

// ------------------------------------------------------------------ units

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateUnit {
    #[serde(rename = "type")]
    pub unit_type: String,
    pub payload: Payload,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateUnit {
    pub payload: Payload,
}

/// The query string of `GET /v1/units`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListUnits {
    #[serde(rename = "type")]
    pub unit_type: Option<String>,
    pub limit: Option<usize>,
    pub after: Option<String>,
}

/// The query string of every change to one unit: the revision it must be at.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    pub expected_revision: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct UnitView {
    pub id: String,
    #[serde(rename = "type")]
    pub unit_type: String,
    /// `active`, `archived` or `deleted`.
    pub lifecycle: &'static str,
    pub revision: u64,
    pub created_at_us: i64,
    pub updated_at_us: i64,
    /// `null` for a deleted unit.
    pub payload: Option<Payload>,
}

#[derive(Debug, Serialize)]
pub struct UnitList {
    pub units: Vec<UnitView>,
    /// Pass it as `after` for the next page; `null` on the last one.
    pub next: Option<String>,
}

// -------------------------------------------------------------- relations

/// Which side of a unit a relation is followed from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Out,
    In,
    #[default]
    Both,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRelation {
    pub from: String,
    pub to: String,
    #[serde(rename = "type")]
    pub relation_type: String,
}

/// The query string of `GET /v1/units/{id}/relations`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListRelations {
    #[serde(default)]
    pub direction: Direction,
    #[serde(rename = "type")]
    pub relation_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RelationView {
    pub id: String,
    pub from: String,
    pub to: String,
    #[serde(rename = "type")]
    pub relation_type: String,
    /// `declared`, `imported`, `algorithmic` or `inferred`.
    pub provenance: &'static str,
    /// Only in a list: `out` when the listed unit is `from`, `in` when it is `to`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
}

#[derive(Debug, Serialize)]
pub struct RelationList {
    pub relations: Vec<RelationView>,
}

// ---------------------------------------------------------------- schemas

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    Bool,
    Int,
    Float,
    Text,
    Bytes,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldDef {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FieldType,
    #[serde(default)]
    pub required: bool,
    /// No two active units of the type hold the same value; implies `indexed`.
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub indexed: bool,
}

/// A unit type's schema: the body of `POST /v1/schemas` and the answer to both schema calls.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schema {
    #[serde(rename = "type")]
    pub unit_type: String,
    pub fields: Vec<FieldDef>,
}

/// `PUT /v1/types/{type}/fulltext` and its reply: whether a unit type is keyword-indexed
/// (off by default, ADR-0023).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fulltext {
    #[serde(rename = "type", default, skip_deserializing)]
    pub unit_type: String,
    pub enabled: bool,
}

// ------------------------------------------------------------------ query

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryRequest {
    pub query: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Access {
    TypeScan,
    FieldIndex { field: String },
}

#[derive(Debug, Serialize)]
pub struct QueryReply {
    pub aliases: Vec<String>,
    /// One unit per alias in each row.
    pub rows: Vec<Vec<UnitView>>,
    pub access: Access,
}

#[derive(Debug, Serialize)]
pub struct Candidate {
    pub access: Access,
    pub cost: u64,
}

#[derive(Debug, Serialize)]
pub struct JoinStep {
    pub alias: String,
    pub unit_type: String,
    pub relation_type: String,
    pub direction: Direction,
    pub from: String,
}

#[derive(Debug, Serialize)]
pub struct ExplainReply {
    pub root_alias: String,
    pub root_type: String,
    pub candidates: Vec<Candidate>,
    /// Index into `candidates` of the path chosen.
    pub chosen: usize,
    pub joins: Vec<JoinStep>,
}

// ----------------------------------------------------------------- search

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Metric {
    #[default]
    Cosine,
    Dot,
}

/// The vector leg. The vectors of the candidate units come with the request: the server keeps no
/// vector index, so the leg ranks exactly the units named here.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VectorLeg {
    pub query: Vec<f32>,
    #[serde(default)]
    pub metric: Metric,
    /// Unit id to its vector.
    pub vectors: BTreeMap<String, Vec<f32>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphLeg {
    pub seeds: Vec<String>,
    #[serde(default)]
    pub direction: Direction,
    pub relation_types: Option<Vec<String>>,
    pub max_depth: Option<u32>,
    pub limit: Option<usize>,
    pub visit_budget: Option<u64>,
}

/// How much each leg counts; 0 switches a leg off, 1 when absent.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Weights {
    pub keyword: Option<u32>,
    pub vector: Option<u32>,
    pub graph: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    pub text: Option<String>,
    pub vector: Option<VectorLeg>,
    pub graph: Option<GraphLeg>,
    pub weights: Option<Weights>,
    pub rrf_k: Option<u32>,
    pub candidates: Option<usize>,
    pub limit: Option<usize>,
}

/// The 1-based rank a unit held in each leg that found it.
#[derive(Debug, Serialize)]
pub struct Ranks {
    pub keyword: Option<u32>,
    pub vector: Option<u32>,
    pub graph: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct Hit {
    pub id: String,
    pub score: u64,
    pub ranks: Ranks,
    pub unit: UnitView,
}

#[derive(Debug, Serialize)]
pub struct SearchReply {
    pub hits: Vec<Hit>,
}

// ----------------------------------------------------------------- health

#[derive(Debug, Serialize)]
pub struct Health {
    pub status: &'static str,
    pub version: &'static str,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn hex_round_trips_and_refuses_what_is_not_hex() {
        assert_eq!(to_hex(&[0, 1, 0xab, 0xff]), "0001abff");
        assert_eq!(from_hex("0001ABff").unwrap(), [0, 1, 0xab, 0xff]);
        assert!(from_hex("abc").is_err());
        assert!(from_hex("zz").is_err());
        assert_eq!(from_hex("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn a_payload_is_exactly_one_of_text_hex_or_fields() {
        let text: Payload = serde_json::from_value(json!({ "text": "hi" })).unwrap();
        assert_eq!(text, Payload::Text("hi".into()));
        let hex: Payload = serde_json::from_value(json!({ "hex": "00ff" })).unwrap();
        assert_eq!(hex, Payload::Hex(Hex(vec![0, 255])));
        assert!(serde_json::from_value::<Payload>(json!({ "text": "hi", "hex": "00" })).is_err());
        assert!(serde_json::from_value::<Payload>(json!({ "blob": "x" })).is_err());
        assert!(serde_json::from_value::<Payload>(json!("hi")).is_err());
    }

    #[test]
    fn a_field_reads_as_the_json_type_it_is() {
        let fields: Payload = serde_json::from_value(json!({
            "fields": { "b": true, "i": 36, "f": 2.5, "t": "x", "y": { "hex": "01" }, "big": 18446744073709551615u64 }
        }))
        .unwrap();
        let Payload::Fields(map) = fields else { panic!("not fields") };
        assert_eq!(map["b"].0, FieldValue::Bool(true));
        assert_eq!(map["i"].0, FieldValue::Int(36));
        assert_eq!(map["f"].0, FieldValue::Float(2.5));
        assert_eq!(map["t"].0, FieldValue::Text("x".into()));
        assert_eq!(map["y"].0, FieldValue::Bytes(vec![1]));
        // Beyond i64: kept as a float, which an `int` field then refuses.
        assert!(matches!(map["big"].0, FieldValue::Float(_)));
    }

    #[test]
    fn a_field_refuses_null_arrays_and_other_objects() {
        for bad in [json!(null), json!([1]), json!({ "hex": 1 }), json!({ "hex": "0" }), json!({ "x": "00" })] {
            assert!(serde_json::from_value::<Field>(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_field_writes_back_as_it_was_read() {
        let values = [json!(true), json!(-4), json!(2.5), json!("x"), json!({ "hex": "0a" })];
        for v in values {
            let field: Field = serde_json::from_value(v.clone()).unwrap();
            assert_eq!(serde_json::to_value(&field).unwrap(), v);
        }
        // A float with no fraction keeps its point, so a reader can tell it from an int.
        assert_eq!(serde_json::to_string(&Field(FieldValue::Float(5.0))).unwrap(), "5.0");
    }

    #[test]
    fn unknown_request_members_are_refused() {
        assert!(
            serde_json::from_value::<CreateUnit>(json!({ "type": "n", "payload": { "text": "x" }, "x": 1 })).is_err()
        );
        assert!(serde_json::from_value::<SearchRequest>(json!({ "txt": "x" })).is_err());
    }

    #[test]
    fn defaults_are_both_directions_and_cosine() {
        assert_eq!(Direction::default(), Direction::Both);
        assert_eq!(Metric::default(), Metric::Cosine);
        let q: ListRelations = serde_json::from_value(json!({})).unwrap();
        assert_eq!(q.direction, Direction::Both);
    }
}
