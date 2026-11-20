//! The text and the JSON of what the commands read, side by side so that one
//! change touches both.

use kerno_units::{Document, FieldValue, Relation, RelationProvenance, Revision, Unit};
use serde_json::{Value, json};

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Bytes as the data they most likely are: UTF-8 text when they are valid, else hex with the length.
pub fn bytes_json(b: &[u8]) -> Value {
    match std::str::from_utf8(b) {
        Ok(text) => json!({ "len": b.len(), "encoding": "utf8", "text": text }),
        Err(_) => json!({ "len": b.len(), "encoding": "hex", "hex": hex(b) }),
    }
}

/// One line of `bytes_json`; `max` cuts the text (in characters) or the hex (in bytes) short.
pub fn bytes_text(b: &[u8], max: usize) -> String {
    match std::str::from_utf8(b) {
        Ok(text) => {
            let shown: String = text.chars().take(max).collect();
            let cut = if shown.len() < text.len() { "…" } else { "" };
            format!("{} bytes, text {:?}{cut}", b.len(), shown)
        }
        Err(_) => {
            let cut = if b.len() > max { "…" } else { "" };
            format!("{} bytes, hex {}{cut}", b.len(), hex(&b[..b.len().min(max)]))
        }
    }
}

fn lossy(b: &Option<Vec<u8>>) -> Value {
    b.as_ref().map_or(Value::Null, |b| Value::String(String::from_utf8_lossy(b).into_owned()))
}

pub fn provenance(p: &RelationProvenance) -> String {
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    match p {
        RelationProvenance::Declared => "declared".into(),
        RelationProvenance::Imported { source } => format!("imported from {}", text(source)),
        RelationProvenance::Algorithmic { origin } => format!("algorithmic by {}", text(origin)),
        RelationProvenance::Inferred { origin, confidence_permille } => {
            format!("inferred by {} at {confidence_permille} permille", text(origin))
        }
    }
}

/// A unit in full: `public` is its public id, when it has one.
pub fn unit(u: &Unit, public: Option<String>) -> (Value, String) {
    let payload = u.payload.as_deref();
    let json = json!({
        "id": u.id.to_string(),
        "public_id": public,
        "type": u.unit_type,
        "lifecycle": format!("{:?}", u.lifecycle),
        "revision": u.head.0,
        "oldest_revision": u.oldest.0,
        "schema_version": u.schema_version,
        "created_seq": u.created_seq.0,
        "updated_seq": u.updated_seq.0,
        "created_at_us": u.created_at.0,
        "updated_at_us": u.updated_at.0,
        "metadata": bytes_json(&u.metadata),
        "payload": payload.map(bytes_json),
    });
    let text = format!(
        "unit       {}\npublic id  {}\ntype       {}\nlifecycle  {:?}\nrevision   {} (oldest retained {})\nschema     {}\n\
         created    commit {} at {}us\nupdated    commit {} at {}us\nmetadata   {}\npayload    {}",
        u.id,
        public.as_deref().unwrap_or("none"),
        u.unit_type,
        u.lifecycle,
        u.head.0,
        u.oldest.0,
        u.schema_version,
        u.created_seq.0,
        u.created_at.0,
        u.updated_seq.0,
        u.updated_at.0,
        bytes_text(&u.metadata, usize::MAX),
        payload.map_or("none".into(), |p| bytes_text(p, usize::MAX)),
    );
    (json, text)
}

/// A unit on one line, for lists.
pub fn unit_line(u: &Unit) -> String {
    let payload = u.payload.as_deref().map_or("no payload".into(), |p| bytes_text(p, 60));
    format!("{}  {}  rev {}  {payload}", u.id, u.unit_type, u.head.0)
}

pub fn revision(r: &Revision) -> (Value, String) {
    let json = json!({
        "revision": r.id.rev.0,
        "kind": format!("{:?}", r.kind),
        "parent": r.parent.map(|p| p.0),
        "commit_seq": r.commit_seq.0,
        "created_at_us": r.created_at.0,
        "lifecycle": format!("{:?}", r.lifecycle),
        "schema_version": r.schema_version,
        "payload_len": r.payload_len,
        "restored_from": r.restored_from.map(|p| p.0),
        "actor": lossy(&r.provenance.actor),
        "source": lossy(&r.provenance.source),
        "metadata": bytes_json(&r.metadata),
        "payload": r.payload.as_deref().map(bytes_json),
    });
    let text = format!(
        "revision {} {:?}  commit {} at {}us  {:?}  schema {}  actor {}  source {}\n  payload {}",
        r.id.rev.0,
        r.kind,
        r.commit_seq.0,
        r.created_at.0,
        r.lifecycle,
        r.schema_version,
        r.provenance.actor.as_ref().map_or("none".into(), |a| String::from_utf8_lossy(a).into_owned()),
        r.provenance.source.as_ref().map_or("none".into(), |a| String::from_utf8_lossy(a).into_owned()),
        // A revision the policy does not keep has no payload to show.
        r.payload.as_deref().map_or(format!("not kept ({} bytes)", r.payload_len), |p| bytes_text(p, 60)),
    );
    (json, text)
}

pub fn relation(r: &Relation, direction: &str) -> (Value, String) {
    let json = json!({
        "direction": direction,
        "id": r.id.to_string(),
        "from": r.from.to_string(),
        "to": r.to.to_string(),
        "type": r.relation_type,
        "created_seq": r.created_seq.0,
        "provenance": provenance(&r.provenance),
    });
    let text = format!(
        "{direction:<3} {}  {} -> {}  {}  commit {}  {}",
        r.id,
        r.from,
        r.to,
        r.relation_type,
        r.created_seq.0,
        provenance(&r.provenance)
    );
    (json, text)
}

fn field_json(v: &FieldValue) -> Value {
    match v {
        FieldValue::Bool(b) => json!(b),
        FieldValue::Int(i) => json!(i),
        FieldValue::Float(f) => json!(f),
        FieldValue::Text(s) => json!(s),
        FieldValue::Bytes(b) => json!({ "hex": hex(b) }),
    }
}

fn field_text(v: &FieldValue) -> String {
    match v {
        FieldValue::Text(s) => format!("{s:?}"),
        FieldValue::Bytes(b) => format!("x\"{}\"", hex(b)),
        FieldValue::Bool(b) => b.to_string(),
        FieldValue::Int(i) => i.to_string(),
        FieldValue::Float(f) => format!("{f:?}"),
    }
}

/// A unit of a query row: its id and type and, when its payload is a record, its fields (else
/// the payload as `unit` shows it).
pub fn fields(u: &Unit) -> (Value, String) {
    let record = u.payload.as_deref().and_then(|p| Document::decode(p).ok());
    match record {
        Some(doc) => {
            let map: serde_json::Map<String, Value> = doc.iter().map(|(k, v)| (k.to_owned(), field_json(v))).collect();
            let shown: Vec<String> = doc.iter().map(|(k, v)| format!("{k}={}", field_text(v))).collect();
            (
                json!({ "id": u.id.to_string(), "type": u.unit_type, "revision": u.head.0, "fields": map }),
                format!("{} {} {{{}}}", u.id, u.unit_type, shown.join(", ")),
            )
        }
        None => {
            // An active unit always has its payload; none reads as empty.
            let payload = u.payload.as_deref().unwrap_or_default();
            (
                json!({ "id": u.id.to_string(), "type": u.unit_type, "revision": u.head.0, "payload": bytes_json(payload) }),
                format!("{} {} {}", u.id, u.unit_type, bytes_text(payload, 60)),
            )
        }
    }
}
