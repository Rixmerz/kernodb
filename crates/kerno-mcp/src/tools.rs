//! The seven tools, built on the public API of `kerno-units` only.
//!
//! A memory is a unit: its text is the payload, its tags are the metadata
//! (newline-joined, as in `examples/agent-memory`). Units are named to the
//! agent by their public id (`k1_...`, ADR-0016): stable across reopen and
//! export, removed when the unit is deleted. That replaces the example's
//! catalog unit, which existed only to hand out short numbers.

use std::collections::BTreeSet;
use std::path::Path;

use kerno_core::vfs::StdVfs;
use kerno_storage::KernoDb;
use kerno_units::{
    AssocDirection, AssocError, AssocQuery, ChangeMeta, Direction, HistoryQuery, MAX_ASSOC_DEPTH, MAX_ASSOC_LIMIT,
    MAX_ASSOC_VISITS, NewUnit, PublicId, RelationProvenance, Unit, UnitConfig, UnitDb, UnitError, UnitFilter, UnitId,
    UnitSnapshot,
};
use serde::Deserialize;
use serde_json::{Value, json};

const DEFAULT_DEPTH: u32 = 2;
const DEFAULT_LIMIT: usize = 20;
const DEFAULT_VISITS: u64 = 10_000;
/// The engine refuses metadata above this many bytes.
const MAX_TAG_BYTES: usize = 4096;

/// Why a tool call failed.
#[derive(Debug)]
pub enum Fail {
    /// The arguments are wrong: a JSON-RPC `invalid params` error.
    Params(String),
    /// The call was well formed and failed: a tool result with `isError`, which
    /// the model can read and react to.
    Tool(String),
}

impl From<UnitError> for Fail {
    fn from(e: UnitError) -> Self {
        Fail::Tool(e.to_string())
    }
}

impl From<AssocError> for Fail {
    fn from(e: AssocError) -> Self {
        match e {
            AssocError::Query(q) => Fail::Params(q.to_string()),
            other => Fail::Tool(other.to_string()),
        }
    }
}

/// The database an MCP session works on.
pub struct Memory {
    db: UnitDb<KernoDb>,
}

impl Memory {
    /// Opens the database in `dir`, creating the directory and the database if
    /// they do not exist. The engine creates the missing directories itself and
    /// syncs their entries, so nothing here may create them first.
    pub fn open(dir: &Path) -> Result<Self, String> {
        let db = UnitDb::<KernoDb>::open(StdVfs::shared(), dir, &UnitConfig::default())
            .map_err(|e| format!("cannot open the database in {}: {e}", dir.display()))?;
        Ok(Memory { db })
    }

    pub fn close(self) -> Result<(), String> {
        self.db.close().map_err(|e| format!("cannot close the database: {e}"))
    }

    /// Runs the tool `name`. The answer is the JSON the agent receives as text.
    pub fn call(&self, name: &str, args: Value) -> Result<Value, Fail> {
        match name {
            "remember" => self.remember(parse(args)?),
            "relate" => self.relate(parse(args)?),
            "recall" => self.recall(parse(args)?),
            "get" => self.get(parse(args)?),
            "history" => self.history(parse(args)?),
            "forget" => self.forget(parse(args)?),
            "verify" => self.verify(parse(args)?),
            other => Err(Fail::Params(format!("unknown tool: {other}"))),
        }
    }
}

fn parse<T: for<'de> Deserialize<'de>>(args: Value) -> Result<T, Fail> {
    // A call without `arguments` is a call with none.
    let args = if args.is_null() { json!({}) } else { args };
    serde_json::from_value(args).map_err(|e| Fail::Params(format!("invalid arguments: {e}")))
}

fn meta(reason: &str) -> ChangeMeta {
    ChangeMeta { reason: Some(reason.to_owned()), actor: Some(b"kerno-mcp".to_vec()), ..ChangeMeta::default() }
}

fn encode_tags(tags: &[String]) -> Result<Vec<u8>, Fail> {
    if tags.iter().any(|t| t.is_empty() || t.contains('\n')) {
        return Err(Fail::Params("a tag is empty or contains a newline".into()));
    }
    let bytes = tags.join("\n").into_bytes();
    if bytes.len() > MAX_TAG_BYTES {
        return Err(Fail::Params(format!("tags take more than {MAX_TAG_BYTES} bytes")));
    }
    Ok(bytes)
}

fn decode_tags(metadata: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(metadata).split('\n').filter(|t| !t.is_empty()).map(str::to_owned).collect()
}

fn text_of(payload: Option<Vec<u8>>) -> Result<String, Fail> {
    String::from_utf8(payload.unwrap_or_default()).map_err(|_| Fail::Tool("a stored text is not UTF-8".into()))
}

/// The unit a public id names. A malformed id is a bad argument; a well formed
/// one that names nothing (never stored, or forgotten) is a failed call.
fn unit_of(snap: &UnitSnapshot<KernoDb>, id: &str) -> Result<UnitId, Fail> {
    let token: PublicId = id.parse().map_err(|e| Fail::Params(format!("id {id:?}: {e}")))?;
    snap.resolve_public_id(&token)?
        .ok_or_else(|| Fail::Tool(format!("no unit {id}: it was never stored, or it was forgotten")))
}

fn name_of(snap: &UnitSnapshot<KernoDb>, unit: UnitId) -> Result<String, Fail> {
    snap.public_id(unit)?
        .map(|t| t.to_string())
        .ok_or_else(|| Fail::Tool(format!("unit {unit} has no public id: it was not created through kerno-mcp")))
}

fn view(snap: &UnitSnapshot<KernoDb>, u: &Unit) -> Result<Value, Fail> {
    Ok(json!({
        "id": name_of(snap, u.id)?,
        "type": u.unit_type,
        "text": text_of(u.payload.clone())?,
        "tags": decode_tags(&u.metadata),
        "revision": u.head.0,
        "created_at_us": u.created_at.0,
        "updated_at_us": u.updated_at.0,
    }))
}

fn check_limit(limit: usize) -> Result<(), Fail> {
    if (1..=MAX_ASSOC_LIMIT).contains(&limit) {
        Ok(())
    } else {
        Err(Fail::Params(format!("limit {limit} is outside 1..={MAX_ASSOC_LIMIT}")))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RememberArgs {
    #[serde(rename = "type")]
    unit_type: String,
    text: String,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RelateArgs {
    from: String,
    to: String,
    relation_type: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Dir {
    Out,
    In,
    Both,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecallArgs {
    from: Option<String>,
    #[serde(rename = "type")]
    unit_type: Option<String>,
    tag: Option<String>,
    direction: Option<Dir>,
    relation_types: Option<Vec<String>>,
    max_depth: Option<u32>,
    visit_budget: Option<u64>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdArgs {
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryArgs {
    id: String,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

impl Memory {
    fn remember(&self, a: RememberArgs) -> Result<Value, Fail> {
        let metadata = encode_tags(&a.tags)?;
        let mut cs = self.db.begin_changeset(meta("remember"))?;
        let (_, id) = cs.create_public(NewUnit {
            unit_type: a.unit_type.clone(),
            schema_version: 1,
            metadata,
            payload: a.text.into_bytes(),
        })?;
        cs.commit()?;
        Ok(json!({ "id": id.to_string(), "type": a.unit_type, "tags": a.tags, "revision": 1 }))
    }

    fn relate(&self, a: RelateArgs) -> Result<Value, Fail> {
        let snap = self.db.snapshot()?;
        let (from, to) = (unit_of(&snap, &a.from)?, unit_of(&snap, &a.to)?);
        let mut cs = self.db.begin_changeset(meta("relate"))?;
        let edge = cs.create_relation(from, to, &a.relation_type, RelationProvenance::Declared)?;
        cs.commit()?;
        Ok(json!({ "relation_id": edge.to_string(), "from": a.from, "to": a.to, "relation_type": a.relation_type }))
    }

    fn recall(&self, a: RecallArgs) -> Result<Value, Fail> {
        let snap = self.db.snapshot()?;
        let limit = a.limit.unwrap_or(DEFAULT_LIMIT);
        match &a.from {
            Some(from) => {
                if a.unit_type.is_some() || a.tag.is_some() {
                    return Err(Fail::Params("use either `from` or `type`/`tag`, not both".into()));
                }
                let seed = unit_of(&snap, from)?;
                let mut q = AssocQuery::new(
                    vec![seed],
                    a.max_depth.unwrap_or(DEFAULT_DEPTH),
                    limit,
                    a.visit_budget.unwrap_or(DEFAULT_VISITS),
                );
                q.direction = match a.direction.unwrap_or(Dir::Both) {
                    Dir::Out => AssocDirection::Out,
                    Dir::In => AssocDirection::In,
                    Dir::Both => AssocDirection::Both,
                };
                q.relation_types = a.relation_types.map(|v| v.into_iter().collect::<BTreeSet<_>>());
                let found = snap.assoc_query(&q)?.result;
                let mut hits = Vec::with_capacity(found.hits.len());
                for hit in &found.hits {
                    let unit = snap
                        .get_unit(hit.unit)?
                        .ok_or_else(|| Fail::Tool(format!("unit {} vanished during the query", hit.unit)))?;
                    let mut path = Vec::with_capacity(hit.path.len());
                    for step in &hit.path {
                        path.push(json!({
                            "relation_id": step.relation.to_string(),
                            "relation_type": step.relation_type,
                            "from": name_of(&snap, step.from)?,
                            "to": name_of(&snap, step.to)?,
                        }));
                    }
                    let mut v = view(&snap, &unit)?;
                    v["distance"] = json!(hit.distance);
                    v["path"] = json!(path);
                    hits.push(v);
                }
                Ok(json!({
                    "hits": hits,
                    "ending": format!("{:?}", found.ending),
                    "relations_examined": found.visited,
                }))
            }
            None => {
                if a.unit_type.is_none() && a.tag.is_none() {
                    return Err(Fail::Params("give `from`, or `type` and/or `tag`".into()));
                }
                if a.direction.is_some()
                    || a.relation_types.is_some()
                    || a.max_depth.is_some()
                    || a.visit_budget.is_some()
                {
                    return Err(Fail::Params(
                        "`direction`, `relation_types`, `max_depth` and `visit_budget` need `from`".into(),
                    ));
                }
                check_limit(limit)?;
                // ponytail: tags live in the unit's metadata, so a tag filter reads every
                // unit of the type; fine for a personal memory, add a tag unit/index if hot.
                // `query` answers active units only: a forgotten one is never listed.
                let filter = UnitFilter { unit_type: a.unit_type.clone(), schema_version: None };
                let mut units: Vec<Unit> = snap
                    .query(&filter)?
                    .units
                    .into_iter()
                    .filter(|u| a.tag.as_ref().is_none_or(|t| decode_tags(&u.metadata).contains(t)))
                    .collect();
                // Newest first, in commit order (the clock is descriptive only).
                units.sort_by_key(|u| std::cmp::Reverse((u.created_seq, u.id)));
                let truncated = units.len() > limit;
                units.truncate(limit);
                let units = units.iter().map(|u| view(&snap, u)).collect::<Result<Vec<_>, _>>()?;
                Ok(json!({ "units": units, "truncated": truncated }))
            }
        }
    }

    fn get(&self, a: IdArgs) -> Result<Value, Fail> {
        let snap = self.db.snapshot()?;
        // `unit_of` already refuses a forgotten unit: deleting removes its public id.
        let unit = snap
            .get_unit(unit_of(&snap, &a.id)?)?
            .ok_or_else(|| Fail::Tool(format!("unit {} has a public id but no record", a.id)))?;
        view(&snap, &unit)
    }

    fn history(&self, a: HistoryArgs) -> Result<Value, Fail> {
        let limit = a.limit.unwrap_or(DEFAULT_LIMIT);
        check_limit(limit)?;
        let snap = self.db.snapshot()?;
        let unit = unit_of(&snap, &a.id)?;
        let query = HistoryQuery { with_payload: true, ..HistoryQuery::latest(limit) };
        let revisions = snap
            .history(unit, query)?
            .into_iter()
            .map(|r| {
                Ok(json!({
                    "revision": r.id.rev.0,
                    "kind": format!("{:?}", r.kind),
                    "created_at_us": r.created_at.0,
                    "text": text_of(r.payload)?,
                    "tags": decode_tags(&r.metadata),
                }))
            })
            .collect::<Result<Vec<Value>, Fail>>()?;
        Ok(json!({ "id": a.id, "revisions": revisions }))
    }

    fn forget(&self, a: IdArgs) -> Result<Value, Fail> {
        let snap = self.db.snapshot()?;
        let unit = unit_of(&snap, &a.id)?;
        // The layer refuses to delete a unit a relation touches (ADR-0014), so the
        // relations go in the same change set. A relation to itself is in both lists.
        let mut edges = BTreeSet::new();
        for dir in [Direction::Out, Direction::In] {
            edges.extend(snap.relations_of(unit, dir, None)?.into_iter().map(|r| r.id));
        }
        let mut cs = self.db.begin_changeset(meta("forget"))?;
        for edge in &edges {
            cs.delete_relation(*edge)?;
        }
        cs.delete(unit, None)?;
        cs.commit()?;
        Ok(json!({
            "id": a.id,
            "forgotten": true,
            "relations_removed": edges.len(),
            "note": "logical deletion: the unit is unreadable now, its bytes stay on disk until compaction (ADR-0009)",
        }))
    }

    fn verify(&self, _: NoArgs) -> Result<Value, Fail> {
        let r = self.db.verify_units()?;
        let findings: Vec<Value> = r
            .findings
            .iter()
            .map(|f| json!({ "rule": f.rule.id(), "class": f.class().to_string(), "detail": f.detail }))
            .collect();
        Ok(json!({
            "clean": r.is_clean(),
            "snapshot_seq": r.seq.0,
            "counts": {
                "units": r.counts.units,
                "revisions": r.counts.revisions,
                "relations": r.counts.relations,
                "change_sets": r.counts.change_sets,
                "public_id_records": r.counts.public_id_records,
            },
            "findings": findings,
            "findings_omitted": r.findings_omitted,
        }))
    }
}

/// The `tools/list` answer: names, descriptions and JSON schemas.
pub fn tool_list() -> Value {
    let id = |what: &str| json!({ "type": "string", "description": what });
    json!([
        {
            "name": "remember",
            "description": "Store a note in long-term memory. Returns its id (k1_...), which every other tool takes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "minLength": 1, "maxLength": 64,
                        "description": "The kind of note, e.g. \"decision\", \"fact\", \"preference\"." },
                    "text": { "type": "string", "description": "The note itself." },
                    "tags": { "type": "array", "items": { "type": "string", "minLength": 1 },
                        "description": "Optional labels; a tag has no newline and all tags together fit 4096 bytes." }
                },
                "required": ["type", "text"],
                "additionalProperties": false
            }
        },
        {
            "name": "relate",
            "description": "Declare a directed relation from one stored unit to another, e.g. \"supersedes\" or \"caused_by\".",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": id("Id of the unit the relation starts at."),
                    "to": id("Id of the unit the relation ends at."),
                    "relation_type": { "type": "string", "minLength": 1, "description": "Name of the relation." }
                },
                "required": ["from", "to", "relation_type"],
                "additionalProperties": false
            }
        },
        {
            "name": "recall",
            "description": format!(
                "Find units. With `from`: follow relations from that unit (an associative query), \
                 nearest first, within the engine's hard bounds (depth <= {MAX_ASSOC_DEPTH}, limit <= \
                 {MAX_ASSOC_LIMIT}, relations examined <= {MAX_ASSOC_VISITS}); `ending` says whether the \
                 answer is complete. Without `from`: list the newest units with the given `type` and/or `tag`."
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": id("Start the associative query at this unit. Not combinable with type/tag."),
                    "type": { "type": "string", "description": "Only units of this type (without `from`)." },
                    "tag": { "type": "string", "description": "Only units carrying this tag (without `from`)." },
                    "direction": { "type": "string", "enum": ["out", "in", "both"], "default": "both",
                        "description": "With `from`: follow relations that start at a unit (out), end at it (in) or either." },
                    "relation_types": { "type": "array", "items": { "type": "string" }, "minItems": 1,
                        "description": "With `from`: follow only these relation types (default: any)." },
                    "max_depth": { "type": "integer", "minimum": 1, "maximum": MAX_ASSOC_DEPTH, "default": DEFAULT_DEPTH,
                        "description": "With `from`: most hops from the start unit." },
                    "visit_budget": { "type": "integer", "minimum": 1, "maximum": MAX_ASSOC_VISITS, "default": DEFAULT_VISITS,
                        "description": "With `from`: most relations the engine may examine." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_ASSOC_LIMIT, "default": DEFAULT_LIMIT,
                        "description": "Most units returned." }
                },
                "additionalProperties": false
            }
        },
        {
            "name": "get",
            "description": "Read one unit at its current revision: type, text, tags, revision number and timestamps.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": id("Id of the unit.") },
                "required": ["id"],
                "additionalProperties": false
            }
        },
        {
            "name": "history",
            "description": "List the retained revisions of a unit, newest first, with the text and tags each one had.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": id("Id of the unit."),
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_ASSOC_LIMIT, "default": DEFAULT_LIMIT,
                        "description": "Most revisions returned." }
                },
                "required": ["id"],
                "additionalProperties": false
            }
        },
        {
            "name": "forget",
            "description": "Forget a unit. Deletion is LOGICAL: the unit stops being readable at once, but its bytes \
                stay on disk until the engine compacts, and this server has no way to erase them on demand (see ADR-0009). \
                Do not use it to remove a secret from disk. Relations that touch the unit are removed with it.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": id("Id of the unit.") },
                "required": ["id"],
                "additionalProperties": false
            }
        },
        {
            "name": "verify",
            "description": "Check the whole database for damage (verify_units) and return its findings. Read-only.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }
    ])
}
