//! Document store: a `document` unit holds the title, and each paragraph of the body is a
//! `chunk` unit with a `part_of` relation to its document. The text is stored once. A search
//! scans the chunks, then reads each hit's document through the relation. Rebuilding a
//! document means reading its chunks back in the order they were made.
//!
//! Run it from the repository root:
//!
//! ```text
//! cargo run -p kerno --example document_store
//! ```

use kerno::{Db, Direction, Error, UnitFilter, UnitId};

const RUNBOOK: &str = "Take a snapshot of the database directory before any upgrade.\n\n\
Backups run every night at 02:00 and are kept for thirty days.\n\n\
To restore, stop the service and copy the latest backup back into place.";

const POLICY: &str = "Personal data is kept for five years.\n\n\
After that it is deleted, and the deletion is logged.";

fn main() -> Result<(), Error> {
    let dir = std::env::temp_dir().join(format!("kerno-example-document-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    // First session: two documents go in.
    let db = Db::open(&dir)?;
    let runbook = store_document(&db, "Backup runbook", RUNBOOK)?;
    store_document(&db, "Retention policy", POLICY)?;
    db.close()?;

    // Second session: search the chunks, then rebuild the document that matched.
    let db = Db::open(&dir)?;
    let hits = search(&db, "backup")?;
    for (title, paragraph) in &hits {
        println!("[{title}] {paragraph}");
    }
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|(title, _)| title == "Backup runbook"));

    let rebuilt = document_text(&db, runbook)?;
    assert_eq!(rebuilt, RUNBOOK);
    println!("rebuilt runbook matches the original: {}", rebuilt == RUNBOOK);

    println!("documents: {}", db.query(&of_type("document"))?.len());
    assert_eq!(db.query(&of_type("document"))?.len(), 2);

    db.close()?;
    std::fs::remove_dir_all(&dir).expect("remove the example database");
    Ok(())
}

/// Stores `title` as a document and each paragraph of `body` as a chunk of it.
fn store_document(db: &Db, title: &str, body: &str) -> Result<UnitId, Error> {
    let document = db.store("document", title)?;
    for paragraph in body.split("\n\n") {
        let chunk = db.store("chunk", paragraph)?;
        db.relate(chunk, document, "part_of")?;
    }
    Ok(document)
}

/// The chunks whose text contains `word` (case-insensitive), with the title of their document.
fn search(db: &Db, word: &str) -> Result<Vec<(String, String)>, Error> {
    let needle = word.to_lowercase();
    let mut hits = Vec::new();
    for chunk in db.query(&of_type("chunk"))? {
        let text = String::from_utf8_lossy(chunk.payload.as_deref().unwrap_or_default()).into_owned();
        if !text.to_lowercase().contains(&needle) {
            continue;
        }
        let part_of = db.related(chunk.id, Direction::Out, Some("part_of"))?;
        let document = db.get(part_of[0].to)?.expect("a chunk belongs to a stored document");
        let title = String::from_utf8_lossy(document.payload.as_deref().unwrap_or_default()).into_owned();
        hits.push((title, text));
    }
    Ok(hits)
}

/// The body of `document`: its chunks joined back with blank lines.
fn document_text(db: &Db, document: UnitId) -> Result<String, Error> {
    let mut paragraphs = Vec::new();
    for edge in db.related(document, Direction::In, Some("part_of"))? {
        let chunk = db.get(edge.from)?.expect("a relation points at a stored chunk");
        paragraphs.push(String::from_utf8_lossy(chunk.payload.as_deref().unwrap_or_default()).into_owned());
    }
    Ok(paragraphs.join("\n\n"))
}

fn of_type(unit_type: &str) -> UnitFilter {
    UnitFilter { unit_type: Some(unit_type.to_owned()), ..UnitFilter::default() }
}
