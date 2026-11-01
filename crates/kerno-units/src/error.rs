//! Errors of the units layer (`etapa3-design.md` §6.6, `units-v1.md` §5).
//!
//! A record that does not decode is a [`DecodeError`]: the first `UNIT-n`
//! rule it breaks (`units-v1.md` §6 order), its class and a deterministic
//! detail that names only record types, fields and numbers, never content
//! (I-29, I-37). Inside [`UnitError`] it becomes
//! `Storage(kerno_core::Error::Corruption | Incompatible)` so engine and
//! layer corruption share one class (design §6.6 rules (a) and (b)).

use std::fmt;

use crate::types::{HistoryPolicy, RevNo, UnitId};

/// Class of a rejected record (`units-v1.md` §5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorClass {
    /// The bytes are not what a writer of this layer produces.
    Corruption,
    /// A newer layer wrote them; nothing is interpreted or modified.
    IncompatibleLayer,
}

impl fmt::Display for ErrorClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ErrorClass::Corruption => "Corruption",
            ErrorClass::IncompatibleLayer => "IncompatibleLayer",
        })
    }
}

/// The structure rules `UNIT-1..=UNIT-25` of `units-v1.md` §5.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnitRule {
    Unit1,
    Unit2,
    Unit3,
    Unit4,
    Unit5,
    Unit6,
    Unit7,
    Unit8,
    Unit9,
    Unit10,
    Unit11,
    Unit12,
    Unit13,
    Unit14,
    Unit15,
    Unit16,
    Unit17,
    Unit18,
    Unit19,
    Unit20,
    Unit21,
    Unit22,
    Unit23,
    Unit24,
    Unit25,
}

impl UnitRule {
    /// Every rule, in numeric order.
    pub const ALL: [UnitRule; 25] = [
        UnitRule::Unit1,
        UnitRule::Unit2,
        UnitRule::Unit3,
        UnitRule::Unit4,
        UnitRule::Unit5,
        UnitRule::Unit6,
        UnitRule::Unit7,
        UnitRule::Unit8,
        UnitRule::Unit9,
        UnitRule::Unit10,
        UnitRule::Unit11,
        UnitRule::Unit12,
        UnitRule::Unit13,
        UnitRule::Unit14,
        UnitRule::Unit15,
        UnitRule::Unit16,
        UnitRule::Unit17,
        UnitRule::Unit18,
        UnitRule::Unit19,
        UnitRule::Unit20,
        UnitRule::Unit21,
        UnitRule::Unit22,
        UnitRule::Unit23,
        UnitRule::Unit24,
        UnitRule::Unit25,
    ];

    /// The `n` of `UNIT-n`.
    pub fn number(self) -> u8 {
        self as u8 + 1
    }

    /// `"UNIT-n"`.
    pub fn id(self) -> String {
        format!("UNIT-{}", self.number())
    }

    /// The class fixed by `units-v1.md` §5: UNIT-2, UNIT-7 and UNIT-16 are
    /// `IncompatibleLayer`, every other rule is `Corruption`.
    pub fn class(self) -> ErrorClass {
        match self {
            UnitRule::Unit2 | UnitRule::Unit7 | UnitRule::Unit16 => ErrorClass::IncompatibleLayer,
            _ => ErrorClass::Corruption,
        }
    }
}

impl fmt::Display for UnitRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UNIT-{}", self.number())
    }
}

/// A rejected key or value: the first rule broken and a deterministic detail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError {
    pub rule: UnitRule,
    pub detail: String,
}

impl DecodeError {
    pub fn new(rule: UnitRule, detail: impl Into<String>) -> Self {
        DecodeError { rule, detail: detail.into() }
    }

    pub fn class(&self) -> ErrorClass {
        self.rule.class()
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}): {}", self.rule, self.class(), self.detail)
    }
}

impl std::error::Error for DecodeError {}

impl From<DecodeError> for kerno_core::Error {
    fn from(e: DecodeError) -> Self {
        match e.class() {
            ErrorClass::Corruption => kerno_core::Error::Corruption(e.to_string()),
            ErrorClass::IncompatibleLayer => kerno_core::Error::Incompatible(e.to_string()),
        }
    }
}

/// Errors of the units layer (design §6.6). Variants that only the
/// operations of E3-03..E3-06 produce are declared now so the type is stable;
/// E3-02 itself produces `Storage` (from [`DecodeError`] and
/// [`crate::PayloadError`]) and `InvalidArgument` (from the encoders).
#[non_exhaustive]
#[derive(Debug)]
pub enum UnitError {
    /// An engine error, or a layer record that failed decoding or payload
    /// verification. The class of the inner error is never changed.
    Storage(kerno_core::Error),
    NotFound {
        unit: UnitId,
    },
    AlreadyExists {
        unit: UnitId,
    },
    Deleted {
        unit: UnitId,
    },
    Conflict {
        unit: UnitId,
        expected: RevNo,
        found: RevNo,
    },
    RevisionNotFound {
        unit: UnitId,
        rev: RevNo,
    },
    Pruned {
        unit: UnitId,
        oldest: RevNo,
    },
    HistoryUnavailable {
        policy: HistoryPolicy,
    },
    PayloadTooLarge {
        len: usize,
        max: usize,
    },
    ChangeSetTooLarge {
        ops: usize,
        bytes: usize,
    },
    InvalidArgument(String),
    NotAUnitDatabase,
    IncompatibleLayer {
        found: u32,
        supported: u32,
    },
    ChangeSetClosed,
}

impl fmt::Display for UnitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnitError::Storage(e) => write!(f, "storage: {e}"),
            UnitError::NotFound { unit } => write!(f, "unit {unit} not found"),
            UnitError::AlreadyExists { unit } => write!(f, "unit {unit} already exists"),
            UnitError::Deleted { unit } => write!(f, "unit {unit} is deleted"),
            UnitError::Conflict { unit, expected, found } => {
                write!(f, "unit {unit}: expected revision {expected}, found {found}")
            }
            UnitError::RevisionNotFound { unit, rev } => write!(f, "unit {unit}: revision {rev} not found"),
            UnitError::Pruned { unit, oldest } => write!(f, "unit {unit}: history pruned before revision {oldest}"),
            UnitError::HistoryUnavailable { policy } => write!(f, "history unavailable under policy {policy:?}"),
            UnitError::PayloadTooLarge { len, max } => write!(f, "payload of {len} bytes exceeds {max}"),
            UnitError::ChangeSetTooLarge { ops, bytes } => {
                write!(f, "change set too large: {ops} operations, {bytes} bytes")
            }
            UnitError::InvalidArgument(m) => write!(f, "invalid argument: {m}"),
            UnitError::NotAUnitDatabase => f.write_str("the database has keys but no units layer mark"),
            UnitError::IncompatibleLayer { found, supported } => {
                write!(f, "layer_version {found} is newer than the supported {supported}")
            }
            UnitError::ChangeSetClosed => f.write_str("the change set is closed"),
        }
    }
}

impl std::error::Error for UnitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            UnitError::Storage(e) => Some(e),
            _ => None,
        }
    }
}

impl From<kerno_core::Error> for UnitError {
    fn from(e: kerno_core::Error) -> Self {
        UnitError::Storage(e)
    }
}

impl From<DecodeError> for UnitError {
    fn from(e: DecodeError) -> Self {
        UnitError::Storage(e.into())
    }
}

impl From<crate::PayloadError> for UnitError {
    fn from(e: crate::PayloadError) -> Self {
        UnitError::Storage(e.into())
    }
}
