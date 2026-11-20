//! The commands. Every one opens the database through the public API of
//! `kerno-storage` and `kerno-units`; the read commands open it read-only
//! (`AccessMode::ReadOnly`: no file is created, synced, deleted or changed,
//! I-18) and a database that does not exist is an error, never a new one.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use kerno_core::vfs::StdVfs;
use kerno_storage::{AccessMode, Config, KernoDb, VerifyOptions};
use kerno_units::{
    Access, AsOf, Direction, FingerprintKey, HistoryQuery, Lifecycle, Plan, PublicId, Query, RelationQuery, UnitConfig,
    UnitDb, UnitFilter, UnitId, UnitSnapshot, UnitVerifyReport,
};
use serde_json::{Value, json};

use crate::{CliError, Opts, Output, render};

type Db = UnitDb<KernoDb>;

fn failure(what: &str, e: impl std::fmt::Display) -> CliError {
    CliError::Failure(format!("{what}: {e}"))
}

/// Opens the database in `dir`, which must exist already.
fn open(dir: &Path, access: AccessMode) -> Result<Db, CliError> {
    let cfg = Config { access, create_if_missing: false, ..Config::default() };
    let what = format!("cannot open {}", dir.display());
    let engine = KernoDb::open(StdVfs::shared(), dir, &cfg).map_err(|e| failure(&what, e))?;
    UnitDb::from_engine(engine, &UnitConfig::default()).map_err(|e| failure(&what, e))
}

/// Runs `work` on the open database and closes it, whatever `work` did.
fn with_db<T>(dir: &Path, access: AccessMode, work: impl FnOnce(&Db) -> Result<T, CliError>) -> Result<T, CliError> {
    let db = open(dir, access)?;
    let done = work(&db);
    let closed = db.close().map_err(|e| failure("cannot close the database", e));
    let done = done?;
    closed?;
    Ok(done)
}

fn read<T>(dir: &Path, work: impl FnOnce(&UnitSnapshot<KernoDb>) -> Result<T, CliError>) -> Result<T, CliError> {
    with_db(dir, AccessMode::ReadOnly, |db| work(&db.snapshot()?))
}

/// A unit named by its 32-digit hex id or by its public id.
fn unit_id(snap: &UnitSnapshot<KernoDb>, name: &str) -> Result<UnitId, CliError> {
    if name.len() == 32 && name.bytes().all(|b| b.is_ascii_hexdigit()) {
        let mut id = [0u8; 16];
        for (byte, pair) in id.iter_mut().zip(name.as_bytes().chunks(2)) {
            *byte = u8::from_str_radix(&String::from_utf8_lossy(pair), 16).unwrap_or(0);
        }
        return Ok(UnitId(id));
    }
    let token: PublicId =
        name.parse().map_err(|_| CliError::Usage(format!("{name:?} is neither a 32-digit hex id nor a public id")))?;
    snap.resolve_public_id(&token)?.ok_or_else(|| CliError::Failure(format!("no unit {name}")))
}

fn existing(snap: &UnitSnapshot<KernoDb>, name: &str) -> Result<kerno_units::Unit, CliError> {
    let id = unit_id(snap, name)?;
    snap.get_unit(id)?.ok_or_else(|| CliError::Failure(format!("no unit {name}")))
}

fn public(snap: &UnitSnapshot<KernoDb>, id: UnitId) -> Result<Option<String>, CliError> {
    Ok(snap.public_id(id)?.map(|t| t.to_string()))
}

// ---------------------------------------------------------------- verify

pub fn verify(dir: &Path, deep: bool) -> Result<Output, CliError> {
    // The engine's check takes the database's lock: it runs before any handle is open.
    let opts = VerifyOptions { deep, check_wal: true };
    let engine = kerno_storage::verify(StdVfs::shared(), dir, &opts).map_err(|e| failure("engine verify", e))?;
    // A store the layer cannot open or read is a finding, not a crash.
    let units: Result<UnitVerifyReport, String> = match open(dir, AccessMode::ReadOnly) {
        Err(CliError::Failure(m) | CliError::Usage(m)) => Err(m),
        Ok(db) => {
            let report = db.verify_units().map_err(|e| format!("verify_units: {e}"));
            let _ = db.close();
            report
        }
    };
    let engine_ok = engine.is_ok();
    let units_ok = units.as_ref().is_ok_and(UnitVerifyReport::is_clean);

    let mut text = format!(
        "engine ({}): {}\n",
        if deep { "deep" } else { "open-time checks; --deep reads every block" },
        if engine_ok { "no findings".into() } else { format!("{} finding(s)", engine.issues.len()) }
    );
    for i in &engine.issues {
        let at = i.offset.map_or(String::new(), |o| format!(" at offset {o}"));
        text.push_str(&format!("  {} {}{at}: {}\n", i.rule, i.file, i.detail));
    }
    text.push_str(&format!("  last commit {}, tables per level {:?}\n", engine.last_seq, level_files(&engine)));
    for o in &engine.orphans {
        text.push_str(&format!(
            "  note: orphan file {} ({} bytes), deleted by the next read-write open\n",
            o.name, o.len
        ));
    }
    match &units {
        Err(m) => text.push_str(&format!("units: cannot be checked: {m}\n")),
        Ok(r) if r.is_clean() => text.push_str(&format!("units: no findings (commit {})\n", r.seq.0)),
        Ok(r) => {
            text.push_str(&format!("units: {} finding(s)\n", r.findings.len() as u64 + r.findings_omitted));
            for f in &r.findings {
                text.push_str(&format!("  {f}\n"));
            }
            if r.findings_omitted > 0 {
                text.push_str(&format!("  ... and {} more not listed\n", r.findings_omitted));
            }
        }
    }
    if let Ok(r) = &units {
        let c = &r.counts;
        text.push_str(&format!(
            "  checked {} unit(s), {} revision(s), {} relation(s), {} index entries\n",
            c.units, c.revisions, c.relations, c.index_entries
        ));
    }
    text.truncate(text.trim_end().len());

    let json = json!({
        "ok": engine_ok && units_ok,
        "deep": deep,
        "engine": {
            "findings": engine.issues.iter().map(|i| json!({
                "rule": i.rule, "file": i.file, "offset": i.offset, "detail": i.detail,
            })).collect::<Vec<_>>(),
            "last_seq": engine.last_seq,
            "levels": engine.levels.iter().map(|l| json!({
                "files": l.files, "bytes": l.bytes, "entries": l.entries, "fragments": l.fragments,
            })).collect::<Vec<_>>(),
            "orphans": engine.orphans.iter().map(|o| o.name.clone()).collect::<Vec<_>>(),
        },
        "units": match &units {
            Err(m) => json!({ "error": m }),
            Ok(r) => json!({
                "seq": r.seq.0,
                "findings": r.findings.iter().map(|f| json!({
                    "rule": f.rule.id(), "class": f.class().to_string(), "detail": f.detail,
                })).collect::<Vec<_>>(),
                "findings_omitted": r.findings_omitted,
                "counts": {
                    "units": r.counts.units, "revisions": r.counts.revisions, "relations": r.counts.relations,
                    "index_entries": r.counts.index_entries,
                },
            }),
        },
    });
    Ok(Output { text, json, failed: !(engine_ok && units_ok) })
}

fn level_files(r: &kerno_storage::VerifyReport) -> Vec<u64> {
    r.levels.iter().map(|l| l.files).collect()
}

// ----------------------------------------------------------------- stats

pub fn stats(dir: &Path) -> Result<Output, CliError> {
    // `inspect` takes the database's lock too: before the handle is opened.
    let info = kerno_storage::inspect(StdVfs::shared(), dir).map_err(|e| failure("cannot inspect the files", e))?;
    let mut formats: Vec<u32> = info.files.iter().filter_map(|f| f.header.map(|h| h.format_version)).collect();
    formats.sort_unstable();
    formats.dedup();

    // ponytail: one `get_unit` per unit and one adjacency scan per unit, linear in the
    // size of the database; the public API has no count by type. Add one to the layer if it is hot.
    let (seq, layer, index, units, relations) = read(dir, |snap| {
        // active, archived, deleted
        let mut units: BTreeMap<String, [u64; 3]> = BTreeMap::new();
        let mut relations: BTreeMap<String, u64> = BTreeMap::new();
        let seq = snap.seq();
        let mut after = None;
        // An empty database has no commit to read as of.
        while seq.0 > 0 || after.is_some() {
            let page = snap.state_as_of(AsOf::Seq(seq), after, 256)?;
            for (id, _) in &page {
                let Some(u) = snap.get_unit(*id)? else { continue };
                let slot = match u.lifecycle {
                    Lifecycle::Active => 0,
                    Lifecycle::Archived => 1,
                    Lifecycle::Deleted => 2,
                };
                units.entry(u.unit_type).or_default()[slot] += 1;
                if u.lifecycle != Lifecycle::Deleted {
                    // Out only: every relation starts at exactly one unit.
                    for r in snap.relations_of(*id, Direction::Out, None)? {
                        *relations.entry(r.relation_type).or_default() += 1;
                    }
                }
            }
            match page.last() {
                Some((id, _)) if page.len() == 256 => after = Some(*id),
                _ => break,
            }
        }
        Ok((seq.0, snap.layer_version()?, snap.index_status()?, units, relations))
    })?;

    let mut files: Vec<(String, u64)> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_file() {
            files.push((entry.file_name().to_string_lossy().into_owned(), meta.len()));
        }
    }
    files.sort();
    let total: u64 = files.iter().map(|f| f.1).sum();

    let mut text = format!(
        "database      {}\nlast commit   {seq}\nlayer version {}\nformat version {}\n",
        dir.display(),
        layer.map_or("none (no layer mark yet)".into(), |v| v.to_string()),
        if formats.is_empty() {
            "none".into()
        } else {
            formats.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
        },
    );
    let live = |c: &[u64; 3]| c[0] + c[1];
    text.push_str(&format!(
        "units         {} live ({} deleted)\n",
        units.values().map(live).sum::<u64>(),
        units.values().map(|c| c[2]).sum::<u64>()
    ));
    for (t, c) in &units {
        text.push_str(&format!("  {t}: {} active, {} archived, {} deleted\n", c[0], c[1], c[2]));
    }
    text.push_str(&format!("relations     {}\n", relations.values().sum::<u64>()));
    for (t, n) in &relations {
        text.push_str(&format!("  {t}: {n}\n"));
    }
    text.push_str(&format!(
        "index         primary {:?}, watermark {}\n",
        index.state,
        index.watermark.map_or("none".into(), |w| w.0.to_string())
    ));
    text.push_str(&format!("files         {} ({total} bytes)\n", files.len()));
    for (name, len) in &files {
        text.push_str(&format!("  {name}  {len}\n"));
    }
    text.truncate(text.trim_end().len());

    let json = json!({
        "dir": dir.display().to_string(),
        "last_seq": seq,
        "layer_version": layer,
        "format_versions": formats,
        "units": {
            "live": units.values().map(live).sum::<u64>(),
            "by_type": units.iter().map(|(t, c)| (t.clone(), json!({
                "active": c[0], "archived": c[1], "deleted": c[2],
            }))).collect::<serde_json::Map<_, _>>(),
        },
        "relations": {
            "total": relations.values().sum::<u64>(),
            "by_type": relations,
        },
        "index": { "state": format!("{:?}", index.state), "watermark": index.watermark.map(|w| w.0) },
        "files": files.iter().map(|(n, l)| json!({ "name": n, "bytes": l })).collect::<Vec<_>>(),
        "total_bytes": total,
    });
    Ok(Output { text, json, failed: false })
}

// ----------------------------------------------------- get, history, relations

pub fn get(dir: &Path, opts: &Opts) -> Result<Output, CliError> {
    read(dir, |snap| {
        let u = existing(snap, &opts.pos[0])?;
        let (json, text) = render::unit(&u, public(snap, u.id)?);
        Ok(Output { text, json, failed: false })
    })
}

pub fn history(dir: &Path, opts: &Opts) -> Result<Output, CliError> {
    read(dir, |snap| {
        let id = unit_id(snap, &opts.pos[0])?;
        let mut revisions = Vec::new();
        let mut query = HistoryQuery { with_payload: true, ..HistoryQuery::oldest(1000) };
        loop {
            let page = snap.history(id, query)?;
            let full = page.len() == query.limit;
            query.from = page.last().map(|r| kerno_units::RevNo(r.id.rev.0 + 1));
            revisions.extend(page);
            if !full {
                break;
            }
        }
        let rendered: Vec<_> = revisions.iter().map(render::revision).collect();
        let text = format!(
            "unit {id}: {} retained revision(s)\n{}",
            rendered.len(),
            rendered.iter().map(|r| r.1.as_str()).collect::<Vec<_>>().join("\n")
        );
        let json =
            json!({ "unit": id.to_string(), "revisions": rendered.into_iter().map(|r| r.0).collect::<Vec<_>>() });
        Ok(Output { text, json, failed: false })
    })
}

pub fn relations(dir: &Path, opts: &Opts) -> Result<Output, CliError> {
    let directions: &[(Direction, &str)] = match opts.value("--dir") {
        None => &[(Direction::Out, "out"), (Direction::In, "in")],
        Some("out") => &[(Direction::Out, "out")],
        Some("in") => &[(Direction::In, "in")],
        Some(other) => return Err(CliError::Usage(format!("relations: --dir is in or out, not {other:?}"))),
    };
    read(dir, |snap| {
        let unit = existing(snap, &opts.pos[0])?.id;
        let mut rows = Vec::new();
        for &(direction, label) in directions {
            let query = RelationQuery { unit, direction, relation_type: opts.value("--type").map(str::to_owned) };
            rows.extend(snap.query_relations(&query)?.relations.iter().map(|r| render::relation(r, label)));
        }
        let text = format!(
            "unit {unit}: {} relation(s)\n{}",
            rows.len(),
            rows.iter().map(|r| r.1.as_str()).collect::<Vec<_>>().join("\n")
        );
        let json = json!({ "unit": unit.to_string(), "relations": rows.into_iter().map(|r| r.0).collect::<Vec<_>>() });
        Ok(Output { text, json, failed: false })
    })
}

// ----------------------------------------------------------------- query

pub fn query(dir: &Path, opts: &Opts) -> Result<Output, CliError> {
    match (opts.pos.first(), opts.value("--type")) {
        (Some(text), None) if !opts.has("--limit") => language(dir, text, opts),
        (Some(_), _) => Err(CliError::Usage(
            "query: give a query text or --type, not both (--limit goes with --type; a query has its own LIMIT)".into(),
        )),
        (None, Some(unit_type)) => by_type(dir, unit_type, opts),
        (None, None) => Err(CliError::Usage("query: give a query text or --type".into())),
    }
}

/// The text and the JSON of a plan of the existing planner, for `--explain`.
fn planner_plan(plan: &Plan) -> (String, Value) {
    let candidates: Vec<Value> = plan
        .candidates()
        .iter()
        .map(
            |c| json!({ "path": format!("{:?}", c.path), "cost": c.cost.0, "usability": format!("{:?}", c.usability) }),
        )
        .collect();
    let mut text = format!(
        "plan {}\n  index {}\n  chosen {:?} because {:?}\n  cache {:?}",
        plan.fingerprint(),
        plan.index().map_or("none".into(), |i| format!("{:?}, watermark {:?}", i.state, i.watermark.map(|w| w.0))),
        plan.chosen().path,
        plan.reason(),
        plan.cache()
    );
    for c in plan.candidates() {
        text.push_str(&format!("\n  candidate {:?} cost {} {:?}", c.path, c.cost.0, c.usability));
    }
    let json = json!({
        "fingerprint": plan.fingerprint().to_string(),
        "index": plan.index().map(|i| json!({ "state": format!("{:?}", i.state), "watermark": i.watermark.map(|w| w.0) })),
        "chosen": format!("{:?}", plan.chosen().path),
        "reason": format!("{:?}", plan.reason()),
        "cache": format!("{:?}", plan.cache()),
        "candidates": candidates,
    });
    (text, json)
}

fn by_type(dir: &Path, unit_type: &str, opts: &Opts) -> Result<Output, CliError> {
    let limit = match opts.value("--limit") {
        None => usize::MAX,
        Some(n) => match n.parse::<usize>() {
            Ok(n) if n >= 1 => n,
            _ => return Err(CliError::Usage(format!("query: --limit is a number of at least 1, not {n:?}"))),
        },
    };
    let filter = UnitFilter { unit_type: Some(unit_type.to_owned()), schema_version: None };
    read(dir, |snap| {
        let plan = if opts.has("--explain") {
            let key = FingerprintKey::generate().map_err(|e| failure("cannot draw the fingerprint key", e))?;
            Some(snap.explain_filter(&key, &filter)?)
        } else {
            None
        };
        let outcome = snap.query(&filter)?;
        let shown: Vec<_> = outcome.units.iter().take(limit).collect();
        let mut text = format!(
            "{} unit(s) of type {unit_type:?}, showing {}; answered by {:?} ({:?})",
            outcome.units.len(),
            shown.len(),
            outcome.path,
            outcome.source
        );
        if let Some(f) = &outcome.fallback {
            text.push_str(&format!("\nfallback: {:?} abandoned ({:?})", f.abandoned, f.cause));
        }
        for u in &shown {
            text.push_str(&format!("\n{}", render::unit_line(u)));
        }
        let mut units = Vec::new();
        for u in &shown {
            units.push(render::unit(u, public(snap, u.id)?).0);
        }
        let mut json = json!({
            "type": unit_type,
            "total": outcome.units.len(),
            "path": format!("{:?}", outcome.path),
            "source": format!("{:?}", outcome.source),
            "fallback": outcome.fallback.map(|f| format!("{f:?}")),
            "units": units,
        });
        if let Some(plan) = plan {
            let (plan_text, plan_json) = planner_plan(&plan);
            text.push_str(&format!("\n{plan_text}"));
            json["plan"] = plan_json;
        }
        Ok(Output { text, json, failed: false })
    })
}

fn access_name(a: &Access) -> String {
    match a {
        Access::TypeScan => "TypeScan".into(),
        Access::FieldIndex { field } => format!("FieldIndex({field})"),
    }
}

/// A query of the language of `docs/query-language.md`.
fn language(dir: &Path, text: &str, opts: &Opts) -> Result<Output, CliError> {
    let query = Query::parse(text).map_err(|e| CliError::Usage(format!("query: {e}")))?;
    read(dir, |snap| {
        let plan = if opts.has("--explain") {
            let key = FingerprintKey::generate().map_err(|e| failure("cannot draw the fingerprint key", e))?;
            Some(snap.explain_query(&key, &query)?)
        } else {
            None
        };
        let result = snap.execute_query(&query)?;
        let mut out = format!(
            "{} row(s) of ({}); first binding read by {}{}",
            result.rows.len(),
            result.aliases.join(", "),
            access_name(&result.access),
            result.scan_path.map_or(String::new(), |p| format!(" through {p:?}")),
        );
        let mut rows = Vec::new();
        for row in &result.rows {
            let cells: Vec<(Value, String)> = row.iter().map(render::fields).collect();
            let line: Vec<String> =
                result.aliases.iter().zip(&cells).map(|(alias, (_, text))| format!("{alias} {text}")).collect();
            out.push_str(&format!("\n{}", line.join("  |  ")));
            rows.push(Value::Array(cells.into_iter().map(|(json, _)| json).collect()));
        }
        let mut json = json!({
            "query": query.to_string(),
            "columns": result.aliases,
            "total": result.rows.len(),
            "access": access_name(&result.access),
            "scan_path": result.scan_path.map(|p| format!("{p:?}")),
            "rows": rows,
        });
        if let Some(plan) = plan {
            let (scan_text, scan_json) = planner_plan(&plan.scan);
            out.push_str(&format!(
                "\nplan: {} ({}), planner v{}",
                plan.root_alias, plan.root_type, plan.planner_version
            ));
            let mut candidates = Vec::new();
            for (i, c) in plan.candidates.iter().enumerate() {
                let mark = if i == plan.chosen { "chosen   " } else { "candidate" };
                out.push_str(&format!("\n  {mark} {} cost {}", access_name(&c.access), c.cost.0));
                candidates
                    .push(json!({ "access": access_name(&c.access), "cost": c.cost.0, "chosen": i == plan.chosen }));
            }
            let mut joins = Vec::new();
            for j in &plan.joins {
                let dir = if j.direction == Direction::Out { "out" } else { "in" };
                out.push_str(&format!(
                    "\n  join {} ({}) via {:?} {dir} from {}: a relation lookup per row, planned",
                    j.alias, j.unit_type, j.relation_type, j.from
                ));
                joins.push(json!({
                    "alias": j.alias, "type": j.unit_type, "relation_type": j.relation_type, "direction": dir, "from": j.from,
                }));
            }
            out.push_str(&format!("\nfirst binding scanned, if it is, through the planner's own plan:\n{scan_text}"));
            json["plan"] = json!({
                "root": { "alias": plan.root_alias, "type": plan.root_type },
                "candidates": candidates,
                "chosen": access_name(&plan.chosen().access),
                "joins": joins,
                "planner_version": plan.planner_version,
                "scan": scan_json,
            });
        }
        Ok(Output { text: out, json, failed: false })
    })
}

// ------------------------------------------------- export, import, reclaim

pub fn export(dir: &Path, opts: &Opts) -> Result<Output, CliError> {
    let file = Path::new(&opts.pos[0]);
    let bytes = with_db(dir, AccessMode::ReadOnly, |db| Ok(kerno_units::export::export(db)?))?;
    // `create_new`: an export never replaces a file that is there.
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(file)
        .map_err(|e| failure(&format!("cannot create {}", file.display()), e))?;
    out.write_all(&bytes)
        .and_then(|()| out.sync_all())
        .map_err(|e| failure(&format!("cannot write {}", file.display()), e))?;
    let text = format!("exported {} bytes to {} (export format v1)", bytes.len(), file.display());
    Ok(Output { text, json: Value::Null, failed: false })
}

pub fn import(opts: &Opts) -> Result<Output, CliError> {
    let (file, new_dir) = (Path::new(&opts.pos[0]), Path::new(&opts.pos[1]));
    let bytes = std::fs::read(file).map_err(|e| failure(&format!("cannot read {}", file.display()), e))?;
    let existed = new_dir.exists();
    if existed && !(new_dir.is_dir() && std::fs::read_dir(new_dir)?.next().is_none()) {
        return Err(CliError::Failure(format!("{} exists and is not an empty directory", new_dir.display())));
    }
    // The engine creates a missing directory itself and syncs its entries.
    let cfg = Config { create_if_missing: true, ..Config::default() };
    let done = KernoDb::open(StdVfs::shared(), new_dir, &cfg)
        .map_err(|e| failure(&format!("cannot create {}", new_dir.display()), e))
        .and_then(|engine| {
            let db = UnitDb::from_engine(engine, &UnitConfig::default())?;
            let report = kerno_units::import::import(&db, &bytes);
            let closed = db.close();
            let report = report?;
            closed?;
            Ok(report)
        });
    match done {
        Ok(r) => Ok(Output {
            text: format!(
                "imported {} unit(s), {} revision(s), {} relation(s), {} polic(ies), {} public id(s) into {}",
                r.units,
                r.revisions,
                r.relations,
                r.policies,
                r.tokens,
                new_dir.display()
            ),
            json: Value::Null,
            failed: false,
        }),
        Err(e) => {
            // A failed import leaves no usable copy (import's contract): take away what this call made.
            let _ = std::fs::remove_dir_all(new_dir);
            if existed {
                let _ = std::fs::create_dir(new_dir);
            }
            Err(e)
        }
    }
}

// ---------------------------------------------------------------- backup

fn store_of(url: &str) -> Result<std::sync::Arc<dyn kerno_backup::ObjectStore>, CliError> {
    kerno_backup::open_store(url).map_err(|e| failure("cannot open the backup location", e))
}

fn backup_failure(e: kerno_backup::Error) -> CliError {
    CliError::Failure(e.to_string())
}

pub fn backup(dir: &Path, opts: &Opts) -> Result<Output, CliError> {
    let url = &opts.pos[0];
    let store = store_of(url)?;
    let info = with_db(dir, AccessMode::ReadOnly, |db| {
        kerno_backup::backup_base(dir, db, &*store, kerno_backup::now_ms()).map_err(backup_failure)
    })?;
    Ok(Output {
        text: format!(
            "backed up commit {}: {} key(s), {} byte(s) in {} part(s) to {url}",
            info.seq, info.entries, info.bytes, info.parts
        ),
        json: json!(info),
        failed: false,
    })
}

pub fn ship(dir: &Path, opts: &Opts) -> Result<Output, CliError> {
    let url = &opts.pos[0];
    let mut shipper = kerno_backup::Shipper::resume(dir, store_of(url)?).map_err(backup_failure)?;
    let r = shipper.ship(kerno_backup::now_ms()).map_err(backup_failure)?;
    Ok(Output {
        text: format!(
            "shipped {} commit(s) in {} segment(s) to {url}; the backup is complete up to commit {}",
            r.commits, r.segments, r.shipped_seq
        ),
        json: json!(r),
        failed: false,
    })
}

pub fn restore(dir: &Path, opts: &Opts) -> Result<Output, CliError> {
    let number = |flag: &str, what: &str| -> Result<Option<u64>, CliError> {
        opts.value(flag)
            .map(|v| v.parse::<u64>().map_err(|_| CliError::Usage(format!("restore: {flag} {v:?} is not {what}"))))
            .transpose()
    };
    let seq = number("--to-seq", "a commit number")?;
    let time = number("--to-time", "a time in milliseconds since the Unix epoch")?;
    let target = match (seq, time) {
        (Some(_), Some(_)) => return Err(CliError::Usage("restore: give --to-seq or --to-time, not both".into())),
        (Some(n), None) => kerno_backup::Target::Seq(n),
        (None, Some(t)) => kerno_backup::Target::Time(t),
        (None, None) => kerno_backup::Target::Latest,
    };
    let r = kerno_backup::restore(&*store_of(&opts.pos[0])?, dir, target).map_err(backup_failure)?;
    Ok(Output {
        text: format!(
            "restored {} at commit {} from the base at commit {} and {} replayed commit(s)",
            dir.display(),
            r.seq,
            r.base_seq,
            r.replayed
        ),
        json: json!(r),
        failed: false,
    })
}

pub fn backups(opts: &Opts) -> Result<Output, CliError> {
    let url = &opts.pos[0];
    let cat = kerno_backup::catalog(&*store_of(url)?).map_err(backup_failure)?;
    let mut text = format!("{url}\n");
    if cat.bases.is_empty() {
        text.push_str("no base backup\n");
    }
    for b in &cat.bases {
        text.push_str(&format!(
            "base at commit {}: taken at {} ms, {} key(s), {} byte(s) in {} part(s); a restore reaches commit {}\n",
            b.seq,
            b.created_ms,
            b.entries,
            b.bytes,
            b.parts,
            cat.reach(b)
        ));
    }
    text.push_str(&format!("{} segment(s) shipped", cat.segments.len()));
    let reach: Vec<u64> = cat.bases.iter().map(|b| cat.reach(b)).collect();
    Ok(Output { text, json: json!({ "bases": cat.bases, "segments": cat.segments, "reach": reach }), failed: false })
}

pub fn reclaim(dir: &Path) -> Result<Output, CliError> {
    with_db(dir, AccessMode::ReadWrite, |db| Ok(db.reclaim()?))?;
    Ok(Output {
        text:
            "live files compacted; this is not a secure erase: nothing is promised about the storage medium (ADR-0009)"
                .into(),
        json: Value::Null,
        failed: false,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use kerno_units::{Candidate, Cost, NormalizedQuery, PlanPath, Usability};

    use super::*;

    /// `--explain` only ever shows a plan of units, which has an index status; the text for a plan
    /// without one (a relation query) is still written down.
    #[test]
    fn a_plan_without_an_index_status_says_none() {
        let key = FingerprintKey::generate().unwrap();
        let query = RelationQuery { unit: UnitId([1; 16]), direction: Direction::Out, relation_type: None };
        let candidate = |path, cost| Candidate { path, cost: Cost(cost), usability: Usability::Usable };
        let plan = Plan::new(
            NormalizedQuery::of_relations(&query).fingerprint(&key),
            None,
            vec![candidate(PlanPath::RelationsReference, 9), candidate(PlanPath::Adjacency, 4)],
        )
        .unwrap();
        let (text, json) = planner_plan(&plan);
        assert!(text.contains("\n  index none\n  chosen Adjacency because LowestCost"), "{text}");
        assert!(text.contains("candidate RelationsReference cost 9") && text.contains("candidate Adjacency cost 4"));
        assert_eq!(json["index"], Value::Null);
        assert_eq!(json["chosen"], "Adjacency");
    }
}
