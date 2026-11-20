//! Chat history on the `kerno` facade, through `Db` methods only.
//!
//! Model: a `session` unit per conversation (schema: unique `name`), one `user_turn` or
//! `assistant_turn` unit per turn (the text is the payload, so it is full-text indexed), and an
//! `in_session` relation from each turn to its session.

use std::collections::HashSet;
use std::path::Path;

use kerno::units::hybrid::{MAX_HYBRID_CANDIDATES, MAX_HYBRID_LIMIT};
use kerno::{Db, Direction, Document, FieldDef, FieldType, FieldValue, HybridQuery};
pub use kerno::{Error, UnitId};

pub const SESSION: &str = "session";
pub const USER_TURN: &str = "user_turn";
pub const ASSISTANT_TURN: &str = "assistant_turn";
pub const IN_SESSION: &str = "in_session";

/// Who said a turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    fn unit_type(self) -> &'static str {
        match self {
            Role::User => USER_TURN,
            Role::Assistant => ASSISTANT_TURN,
        }
    }

    fn from_unit_type(unit_type: &str) -> Option<Role> {
        match unit_type {
            USER_TURN => Some(Role::User),
            ASSISTANT_TURN => Some(Role::Assistant),
            _ => None,
        }
    }
}

/// One turn of a conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    pub id: UnitId,
    pub role: Role,
    pub text: String,
}

/// A page of turns, oldest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub turns: Vec<Turn>,
    /// Pass this as `before` to [`ChatHistory::recent`] for the previous page; `None` when no
    /// older turn exists.
    pub older: Option<UnitId>,
}

pub struct ChatHistory {
    db: Db,
}

impl ChatHistory {
    /// Opens (or creates) the history in `dir`.
    ///
    /// # Errors
    ///
    /// The database cannot be opened or its schema cannot be defined.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, Error> {
        let db = Db::open(dir)?;
        // A type keeps one schema for good, so define it only the first time.
        if db.schema(SESSION)?.is_none() {
            db.define_schema(SESSION, vec![FieldDef::new("name", FieldType::Text).unique().required()])?;
        }
        Ok(ChatHistory { db })
    }

    /// The session called `name`, created when it does not exist.
    ///
    /// # Errors
    ///
    /// The engine fails.
    pub fn session(&self, name: &str) -> Result<UnitId, Error> {
        let name = FieldValue::Text(name.to_owned());
        if let Some(found) = self.db.find(SESSION, "name", &name)?.first() {
            return Ok(found.id);
        }
        self.db.store_document(SESSION, &Document::new().with("name", name))
    }

    /// Appends a turn to `session` and returns its id.
    ///
    /// Two commits (the turn, then its relation). A crash between them leaves an orphan turn,
    /// which is harmless: [`recent`](Self::recent) and [`search`](Self::search) only reach turns
    /// through `in_session`.
    ///
    /// # Errors
    ///
    /// The session does not exist or the engine fails.
    pub fn append(&self, session: UnitId, role: Role, text: &str) -> Result<UnitId, Error> {
        let turn = self.db.store(role.unit_type(), text)?;
        self.db.relate(turn, session, IN_SESSION)?;
        Ok(turn)
    }

    /// Up to `limit` turns of `session`, ending just before the turn `before` (the newest turns
    /// when `None`), oldest first.
    ///
    /// Costs one read of every turn relation of the session.
    ///
    /// # Errors
    ///
    /// `limit` is 0, `before` is not a turn of this session, or the engine fails.
    pub fn recent(&self, session: UnitId, limit: usize, before: Option<UnitId>) -> Result<Page, Error> {
        if limit == 0 {
            return Err(Error::InvalidArgument("limit must be at least 1".into()));
        }
        let mut edges = self.db.related(session, Direction::In, Some(IN_SESSION))?;
        // The commit sequence persists and only grows; relation ids order only within a process.
        edges.sort_by_key(|r| (r.created_seq, r.id));
        let end = match before {
            None => edges.len(),
            Some(b) => edges
                .iter()
                .position(|r| r.from == b)
                .ok_or_else(|| Error::InvalidArgument(format!("turn {b} is not in this session")))?,
        };
        let start = end.saturating_sub(limit);
        let turns = edges[start..end].iter().map(|r| self.load(r.from)).collect::<Result<_, _>>()?;
        Ok(Page { turns, older: (start > 0).then(|| edges[start].from) })
    }

    /// The turns of `session` that match `words`, best first, at most `limit`.
    ///
    /// Keyword search is global (BM25 over every unit's text, at most the best 1000), so a match
    /// of this session ranked below that is missed.
    ///
    /// # Errors
    ///
    /// `words` is empty, or the engine fails.
    pub fn search(&self, session: UnitId, words: &str, limit: usize) -> Result<Vec<Turn>, Error> {
        let members: HashSet<UnitId> =
            self.db.related(session, Direction::In, Some(IN_SESSION))?.iter().map(|r| r.from).collect();
        let query =
            HybridQuery { text: Some(words), candidates: MAX_HYBRID_CANDIDATES, ..HybridQuery::new(MAX_HYBRID_LIMIT) };
        self.db
            .hybrid_search(&query)?
            .iter()
            .filter(|hit| members.contains(&hit.unit))
            .take(limit)
            .map(|hit| self.load(hit.unit))
            .collect()
    }

    /// Closes the database.
    ///
    /// # Errors
    ///
    /// The engine fails to close cleanly.
    pub fn close(self) -> Result<(), Error> {
        self.db.close()
    }

    fn load(&self, id: UnitId) -> Result<Turn, Error> {
        let unit = self.db.get(id)?.expect("a turn relation starts at a stored turn");
        let role = Role::from_unit_type(&unit.unit_type).expect("a turn has a turn type");
        let text = String::from_utf8(unit.payload.unwrap_or_default()).expect("a turn is stored as UTF-8");
        Ok(Turn { id, role, text })
    }
}
