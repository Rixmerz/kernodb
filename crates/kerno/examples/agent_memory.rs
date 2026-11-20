//! Agent memory: each fact an agent learns is a `memory` unit, each subject is a `topic`
//! unit, and an `about` relation links a fact to its topics. Recall follows the relations,
//! and the memory is still there after the database is closed and opened again.
//!
//! Run it from the repository root:
//!
//! ```text
//! cargo run -p kerno --example agent_memory
//! ```

use kerno::{Db, Direction, Error, UnitFilter, UnitId};

fn main() -> Result<(), Error> {
    let dir = std::env::temp_dir().join(format!("kerno-example-agent-memory-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    // First session: the agent learns three facts.
    let db = Db::open(&dir)?;
    remember(&db, "The staging deploy runs on Fridays at 17:00.", &["deploy"])?;
    remember(&db, "Tokens expire after 15 minutes.", &["auth"])?;
    remember(&db, "Rollback is done with the previous image tag.", &["deploy"])?;
    db.close()?;

    // Second session: the facts survived the close, and recall follows the relations.
    let db = Db::open(&dir)?;
    let deploy = recall(&db, "deploy")?;
    println!("about deploy:");
    for fact in &deploy {
        println!("  - {fact}");
    }
    assert_eq!(deploy.len(), 2);
    assert!(deploy.iter().any(|f| f.contains("Fridays")));

    let auth = recall(&db, "auth")?;
    assert_eq!(auth, ["Tokens expire after 15 minutes."]);
    println!("about auth:");
    println!("  - {}", auth[0]);

    let all = db.query(&of_type("memory"))?;
    println!("memories stored: {}", all.len());
    assert_eq!(all.len(), 3);

    db.close()?;
    std::fs::remove_dir_all(&dir).expect("remove the example database");
    Ok(())
}

/// Stores `text` as a memory and relates it to each named topic, creating a topic the
/// first time it is named.
fn remember(db: &Db, text: &str, topics: &[&str]) -> Result<UnitId, Error> {
    let memory = db.store("memory", text)?;
    for name in topics {
        let topic = match find_topic(db, name)? {
            Some(id) => id,
            None => db.store("topic", *name)?,
        };
        db.relate(memory, topic, "about")?;
    }
    Ok(memory)
}

/// The text of every memory related to the topic `name`, in the order they were related.
fn recall(db: &Db, name: &str) -> Result<Vec<String>, Error> {
    let Some(topic) = find_topic(db, name)? else {
        return Ok(Vec::new());
    };
    let mut texts = Vec::new();
    for edge in db.related(topic, Direction::In, Some("about"))? {
        let memory = db.get(edge.from)?.expect("a relation points at a stored memory");
        texts.push(String::from_utf8_lossy(memory.payload.as_deref().unwrap_or_default()).into_owned());
    }
    Ok(texts)
}

fn find_topic(db: &Db, name: &str) -> Result<Option<UnitId>, Error> {
    let topics = db.query(&of_type("topic"))?;
    Ok(topics.into_iter().find(|t| t.payload.as_deref() == Some(name.as_bytes())).map(|t| t.id))
}

fn of_type(unit_type: &str) -> UnitFilter {
    UnitFilter { unit_type: Some(unit_type.to_owned()), ..UnitFilter::default() }
}
