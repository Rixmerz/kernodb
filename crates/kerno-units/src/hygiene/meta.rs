//! The importance score and the TTL of a unit, carried in its `metadata`.

use kerno_core::contract::TxnEngine;

use crate::db::{ChangeSet, Unit, UnitUpdate};
use crate::error::UnitError;
use crate::types::{Lifecycle, RevNo, Timestamp, UnitId};

/// Importance is in thousandths, `0..=1000` (the scale of
/// [`crate::RelationProvenance::Inferred`]'s confidence).
pub const MAX_IMPORTANCE: u16 = 1000;

const MAGIC: &[u8; 4] = b"KHYG";
const FORMAT: u8 = 1;
/// Marker, format, importance, has-ttl flag.
const FIXED: usize = 4 + 1 + 2 + 1;

/// What hygiene keeps in a unit's metadata: an importance score, an optional
/// expiry instant and the caller's own metadata bytes. The metadata is
/// `"KHYG" 1 importance:u16be flag:u8 [expires_at:i64be] user..`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hygiene {
    /// `0..=`[`MAX_IMPORTANCE`]; 0 for a unit that never had one.
    pub importance: u16,
    /// The unit is due for [`crate::UnitDb::expire_due`] once the injected
    /// clock reaches this instant. Descriptive time, like `created_at`
    /// (I-26): it is only compared with the clock, never used for ordering.
    pub expires_at: Option<Timestamp>,
    /// The caller's own metadata, kept as it was.
    pub user: Vec<u8>,
}

impl Hygiene {
    /// The metadata bytes for [`crate::NewUnit::metadata`] or
    /// [`crate::UnitUpdate::metadata`].
    pub fn encode(&self) -> Result<Vec<u8>, UnitError> {
        if self.importance > MAX_IMPORTANCE {
            return Err(UnitError::InvalidArgument(format!(
                "importance {} is above {MAX_IMPORTANCE}",
                self.importance
            )));
        }
        let mut out = Vec::with_capacity(FIXED + 8 + self.user.len());
        out.extend_from_slice(MAGIC);
        out.push(FORMAT);
        out.extend_from_slice(&self.importance.to_be_bytes());
        match self.expires_at {
            Some(t) => {
                out.push(1);
                out.extend_from_slice(&t.0.to_be_bytes());
            }
            None => out.push(0),
        }
        out.extend_from_slice(&self.user);
        Ok(out)
    }

    /// The hygiene data in `metadata`; `None` when it does not start with the
    /// marker (plain metadata). A marker followed by anything this module
    /// would not write is an error: a unit is never silently treated as
    /// having no TTL.
    pub fn decode(metadata: &[u8]) -> Result<Option<Hygiene>, UnitError> {
        if !metadata.starts_with(MAGIC) {
            return Ok(None);
        }
        let bad = |what: &str| UnitError::InvalidArgument(format!("hygiene metadata: {what}"));
        let Some(fixed) = metadata.get(..FIXED) else {
            return Err(bad("shorter than its header"));
        };
        if fixed[4] != FORMAT {
            return Err(bad("unknown format"));
        }
        let importance = u16::from_be_bytes([fixed[5], fixed[6]]);
        if importance > MAX_IMPORTANCE {
            return Err(bad("importance above the maximum"));
        }
        let (expires_at, rest) = match fixed[7] {
            0 => (None, &metadata[FIXED..]),
            1 => {
                let Some(raw) = metadata.get(FIXED..FIXED + 8) else {
                    return Err(bad("TTL cut short"));
                };
                let mut b = [0u8; 8];
                b.copy_from_slice(raw);
                (Some(Timestamp(i64::from_be_bytes(b))), &metadata[FIXED + 8..])
            }
            _ => return Err(bad("unknown TTL flag")),
        };
        Ok(Some(Hygiene { importance, expires_at, user: rest.to_vec() }))
    }

    /// The hygiene data of `unit`: [`Hygiene::default`] for plain metadata.
    pub fn of(unit: &Unit) -> Result<Hygiene, UnitError> {
        Ok(Self::decode(&unit.metadata)?
            .unwrap_or_else(|| Hygiene { user: unit.metadata.clone(), ..Hygiene::default() }))
    }

    /// Whether the TTL has passed at `now`.
    pub fn is_expired(&self, now: Timestamp) -> bool {
        self.expires_at.is_some_and(|t| t <= now)
    }
}

impl<E: TxnEngine> ChangeSet<'_, E> {
    /// Sets the importance and the TTL of an `Active` unit: a new revision
    /// with the same payload and the caller's own metadata kept. Both are
    /// replaced, so pass the current value to keep one.
    pub fn set_hygiene(
        &mut self,
        id: UnitId,
        importance: u16,
        expires_at: Option<Timestamp>,
    ) -> Result<RevNo, UnitError> {
        self.guarded(|s| s.set_hygiene_unguarded(id, importance, expires_at))
    }

    fn set_hygiene_unguarded(
        &mut self,
        id: UnitId,
        importance: u16,
        expires_at: Option<Timestamp>,
    ) -> Result<RevNo, UnitError> {
        let Some(unit) = self.get_unit(id)? else {
            return Err(UnitError::NotFound { unit: id });
        };
        if unit.lifecycle == Lifecycle::Deleted {
            return Err(UnitError::Deleted { unit: id });
        }
        let Some(payload) = unit.payload.clone() else {
            return Err(UnitError::InvalidArgument(format!("unit {id} has no payload")));
        };
        let metadata = Hygiene { importance, expires_at, ..Hygiene::of(&unit)? }.encode()?;
        self.update(id, UnitUpdate { payload, schema_version: None, metadata: Some(metadata) }, Some(unit.head))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for h in [
            Hygiene::default(),
            Hygiene { importance: 700, expires_at: None, user: b"mine".to_vec() },
            Hygiene { importance: MAX_IMPORTANCE, expires_at: Some(Timestamp(-5)), user: Vec::new() },
        ] {
            assert_eq!(Hygiene::decode(&h.encode().unwrap()).unwrap(), Some(h));
        }
    }

    #[test]
    fn plain_metadata_is_not_hygiene() {
        assert_eq!(Hygiene::decode(b"").unwrap(), None);
        assert_eq!(Hygiene::decode(b"KHY").unwrap(), None);
        assert_eq!(Hygiene::decode(b"hello").unwrap(), None);
    }

    #[test]
    fn refuses_what_it_would_not_write() {
        assert!(Hygiene { importance: MAX_IMPORTANCE + 1, ..Hygiene::default() }.encode().is_err());
        let good = Hygiene { importance: 1, expires_at: Some(Timestamp(9)), user: vec![] }.encode().unwrap();
        assert!(Hygiene::decode(&good[..FIXED - 1]).is_err(), "short header");
        assert!(Hygiene::decode(&good[..FIXED + 3]).is_err(), "cut TTL");
        let mut v = good.clone();
        v[4] = 2;
        assert!(Hygiene::decode(&v).is_err(), "format");
        let mut v = good.clone();
        v[7] = 2;
        assert!(Hygiene::decode(&v).is_err(), "flag");
        let mut v = good;
        v[5..7].copy_from_slice(&(MAX_IMPORTANCE + 1).to_be_bytes());
        assert!(Hygiene::decode(&v).is_err(), "importance");
    }

    #[test]
    fn expiry_is_inclusive() {
        let h = Hygiene { expires_at: Some(Timestamp(10)), ..Hygiene::default() };
        assert!(!h.is_expired(Timestamp(9)));
        assert!(h.is_expired(Timestamp(10)));
        assert!(!Hygiene::default().is_expired(Timestamp(i64::MAX)));
    }
}
