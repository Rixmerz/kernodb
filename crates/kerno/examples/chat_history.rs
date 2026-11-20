//! Chat history: each message is a unit whose type says who wrote it (`user_message` or
//! `assistant_message`), each conversation is a `conversation` unit, and an
//! `in_conversation` relation links every message to its conversation. An answer is also
//! related to the question it answers. Replaying a conversation means reading its relations
//! in the order they were made.
//!
//! Run it from the repository root:
//!
//! ```text
//! cargo run -p kerno --example chat_history
//! ```

use kerno::{Db, Direction, Error, UnitFilter, UnitId};

fn main() -> Result<(), Error> {
    let dir = std::env::temp_dir().join(format!("kerno-example-chat-history-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    // First session: one conversation, written as it happens.
    let db = Db::open(&dir)?;
    let conversation = db.store("conversation", "Kerno questions")?;
    let question = message(&db, conversation, "user_message", "Does Kerno need a server?", None)?;
    message(&db, conversation, "assistant_message", "No: it is an embedded library.", Some(question))?;
    let question = message(&db, conversation, "user_message", "Can two processes open it?", None)?;
    message(&db, conversation, "assistant_message", "One process holds a directory at a time.", Some(question))?;
    db.close()?;

    // Second session: the history is read back in order.
    let db = Db::open(&dir)?;
    let transcript = replay(&db, conversation)?;
    for (unit_type, text) in &transcript {
        println!("{unit_type:<19} {text}");
    }
    assert_eq!(transcript.len(), 4);
    assert_eq!(transcript[0].0, "user_message");
    assert_eq!(transcript[1].0, "assistant_message");
    assert_eq!(transcript[2].1, "Can two processes open it?");

    // Each answer points at the question it answers.
    let answers = db.query(&of_type("assistant_message"))?;
    for answer in &answers {
        let asked = db.related(answer.id, Direction::Out, Some("answers"))?;
        assert_eq!(asked.len(), 1);
    }
    println!("answers linked to their question: {}", answers.len());
    assert_eq!(answers.len(), 2);

    db.close()?;
    std::fs::remove_dir_all(&dir).expect("remove the example database");
    Ok(())
}

/// Stores one message in `conversation`, and links it to `answers` when it is a reply.
fn message(db: &Db, conversation: UnitId, role: &str, text: &str, answers: Option<UnitId>) -> Result<UnitId, Error> {
    let id = db.store(role, text)?;
    db.relate(id, conversation, "in_conversation")?;
    if let Some(question) = answers {
        db.relate(id, question, "answers")?;
    }
    Ok(id)
}

/// The messages of `conversation` as (type, text), in the order they were written.
fn replay(db: &Db, conversation: UnitId) -> Result<Vec<(String, String)>, Error> {
    let mut transcript = Vec::new();
    for edge in db.related(conversation, Direction::In, Some("in_conversation"))? {
        let unit = db.get(edge.from)?.expect("a relation points at a stored message");
        let text = String::from_utf8_lossy(unit.payload.as_deref().unwrap_or_default()).into_owned();
        transcript.push((unit.unit_type, text));
    }
    Ok(transcript)
}

fn of_type(unit_type: &str) -> UnitFilter {
    UnitFilter { unit_type: Some(unit_type.to_owned()), ..UnitFilter::default() }
}
