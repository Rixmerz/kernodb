//! Value types of the layer (`etapa3-design.md` §6.2) and the enumerations
//! of `units-v1.md` §3. Every enumeration maps to its byte both ways; a byte
//! outside the table has no value (`None`), so it is never interpreted.

use std::fmt;

/// Internal unit identifier: 16 opaque bytes (D-05).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnitId(pub [u8; 16]);

impl UnitId {
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for UnitId {
    /// Lower-case hex, 32 digits: ids may appear in messages (I-29 allows ids).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

/// Revision number of a unit, 1-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RevNo(pub u64);

/// An engine `commit_sequence`, 1-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommitSeq(pub u64);

/// A change set's id: it is its `commit_sequence` (design §2.5).
pub type ChangeSetId = CommitSeq;

/// Microseconds since the Unix epoch. Descriptive only (I-26).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(pub i64);

/// Public revision id: `(unit_id, rev_no)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RevisionId {
    pub unit: UnitId,
    pub rev: RevNo,
}

macro_rules! display_inner {
    ($($t:ty),*) => {$(
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    )*};
}
display_inner!(RevNo, CommitSeq, Timestamp);

macro_rules! byte_enum {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $v:ident = $b:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name { $($(#[$vm])* $v),+ }

        impl $name {
            /// Every variant, in byte order.
            pub const ALL: &'static [$name] = &[$($name::$v),+];

            /// The variant stored as `b`, or `None` outside the table.
            pub fn from_byte(b: u8) -> Option<Self> {
                match b {
                    $($b => Some($name::$v),)+
                    _ => None,
                }
            }

            pub fn to_byte(self) -> u8 {
                match self {
                    $($name::$v => $b,)+
                }
            }
        }
    };
}

byte_enum!(
    /// `UNIT_HEAD.lifecycle`.
    Lifecycle { Active = 0, Archived = 1, Deleted = 2 }
);

byte_enum!(
    /// `REV_META.kind` and `CS_MEMBER.kind`. `0` and `7..=255` have no value
    /// (`7..=15` are reserved for later layer versions).
    RevisionKind { Create = 1, Update = 2, Archive = 3, Unarchive = 4, Delete = 5, Restore = 6 }
);

/// The name the design uses (§6.2).
pub type ChangeKind = RevisionKind;

byte_enum!(
    /// `CHANGESET.purpose`.
    Purpose { Application = 0, Retention = 1, Maintenance = 2 }
);

/// The name the design uses (§6.3).
pub type ChangePurpose = Purpose;

byte_enum!(
    /// `LAYER_META.history_policy` (its semantics are E3-05's).
    HistoryPolicy { None = 0, Audit = 1, Versioned = 2 }
);
