//! Atomic write batches and the size limits shared by every engine.

use crate::error::{Error, Result};

/// Upper bound for key length accepted by every candidate. It also bounds
/// both ends of an [`Op::DeleteRange`].
pub const MAX_KEY_LEN: usize = 512;
/// Upper bound for value length accepted by every candidate.
pub const MAX_VALUE_LEN: usize = 1 << 20;

/// One operation of a [`WriteBatch`]. `DeleteRange { start, end }` deletes
/// every key `k` with `start <= k < end`; `start == end` deletes nothing and
/// `start > end` is rejected by [`WriteBatch::validate`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
    DeleteRange { start: Vec<u8>, end: Vec<u8> },
}

impl Op {
    /// The key of a point operation, or the start of a range deletion.
    /// Callers that index or deduplicate operations by key must handle
    /// [`Op::DeleteRange`] separately: its effect is not limited to `start`.
    pub fn key(&self) -> &[u8] {
        match self {
            Op::Put { key, .. } | Op::Delete { key } => key,
            Op::DeleteRange { start, .. } => start,
        }
    }
}

/// An atomic group of writes.
///
/// Operations apply in order: a later operation on a key wins over an
/// earlier one, a [`delete_range`](Self::delete_range) deletes the keys put
/// earlier in the same batch, and a key put after it in the same batch
/// survives it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriteBatch {
    ops: Vec<Op>,
}

impl WriteBatch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&mut self, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) -> &mut Self {
        self.ops.push(Op::Put { key: key.into(), value: value.into() });
        self
    }

    pub fn delete(&mut self, key: impl Into<Vec<u8>>) -> &mut Self {
        self.ops.push(Op::Delete { key: key.into() });
        self
    }

    /// Deletes every key in `[start, end)`. `start == end` is accepted and
    /// deletes nothing (a batch made only of such operations is still a
    /// non-empty commit and gets a sequence number); `start > end` makes the
    /// batch invalid.
    pub fn delete_range(&mut self, start: impl Into<Vec<u8>>, end: impl Into<Vec<u8>>) -> &mut Self {
        self.ops.push(Op::DeleteRange { start: start.into(), end: end.into() });
        self
    }

    /// Whether any operation is an [`Op::DeleteRange`]. Engines that do not
    /// implement range deletions reject such a batch before writing anything.
    pub fn has_range_deletes(&self) -> bool {
        self.ops.iter().any(|op| matches!(op, Op::DeleteRange { .. }))
    }

    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Sum of key and value bytes ("logical bytes" for amplification math);
    /// a range deletion counts both of its bounds.
    pub fn payload_bytes(&self) -> usize {
        self.ops
            .iter()
            .map(|op| match op {
                Op::Put { key, value } => key.len() + value.len(),
                Op::Delete { key } => key.len(),
                Op::DeleteRange { start, end } => start.len() + end.len(),
            })
            .sum()
    }

    /// Checks the size limits shared by every candidate (both bounds of a
    /// range deletion are keys), and that no range deletion has
    /// `start > end`.
    pub fn validate(&self) -> Result<()> {
        for op in &self.ops {
            match op {
                Op::Put { key, .. } | Op::Delete { key } => check_key_len("key", key)?,
                Op::DeleteRange { start, end } => {
                    check_key_len("range start", start)?;
                    check_key_len("range end", end)?;
                    if start > end {
                        return Err(Error::InvalidArgument("delete_range with start > end".into()));
                    }
                }
            }
            if let Op::Put { value, .. } = op
                && value.len() > MAX_VALUE_LEN
            {
                return Err(Error::InvalidArgument(format!(
                    "value of {} bytes exceeds MAX_VALUE_LEN ({MAX_VALUE_LEN})",
                    value.len()
                )));
            }
        }
        Ok(())
    }
}

fn check_key_len(what: &str, key: &[u8]) -> Result<()> {
    if key.len() > MAX_KEY_LEN {
        return Err(Error::InvalidArgument(format!(
            "{what} of {} bytes exceeds MAX_KEY_LEN ({MAX_KEY_LEN})",
            key.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invalid(b: &WriteBatch) -> bool {
        matches!(b.validate(), Err(Error::InvalidArgument(_)))
    }

    #[test]
    fn delete_range_is_recorded_in_order() {
        let mut b = WriteBatch::new();
        b.put("a", "1").delete_range("a", "c").delete("d");
        assert_eq!(
            b.ops(),
            &[
                Op::Put { key: b"a".to_vec(), value: b"1".to_vec() },
                Op::DeleteRange { start: b"a".to_vec(), end: b"c".to_vec() },
                Op::Delete { key: b"d".to_vec() },
            ]
        );
        assert_eq!(b.len(), 3);
        assert!(b.has_range_deletes());
        assert_eq!(b.ops()[1].key(), b"a", "key() of a range deletion is its start");
        let mut points = WriteBatch::new();
        points.put("a", "1").delete("b");
        assert!(!points.has_range_deletes());
        assert!(!WriteBatch::new().has_range_deletes());
    }

    #[test]
    fn payload_bytes_counts_both_range_bounds() {
        let mut b = WriteBatch::new();
        b.put("ab", "xyz").delete("q").delete_range("start", "end-key");
        assert_eq!(b.payload_bytes(), 2 + 3 + 1 + 5 + 7);
    }

    #[test]
    fn validate_checks_range_bounds() {
        let max = vec![b'k'; MAX_KEY_LEN];
        let over = vec![b'k'; MAX_KEY_LEN + 1];
        let mut ok = WriteBatch::new();
        ok.delete_range("a", "b").delete_range("same", "same").delete_range(b"".to_vec(), max.clone());
        ok.delete_range(max.clone(), max.clone());
        assert!(ok.validate().is_ok(), "start <= end within MAX_KEY_LEN is valid, including start == end");

        let mut b = WriteBatch::new();
        b.put("x", "1").delete_range("b", "a");
        assert!(invalid(&b), "start > end");
        let mut b = WriteBatch::new();
        b.delete_range(over.clone(), over.clone());
        assert!(invalid(&b), "bounds longer than MAX_KEY_LEN, even when the range is empty");
        let mut b = WriteBatch::new();
        b.delete_range("a", over.clone());
        assert!(invalid(&b), "end longer than MAX_KEY_LEN");
        let mut b = WriteBatch::new();
        b.delete_range(over, "zzz");
        assert!(invalid(&b), "start longer than MAX_KEY_LEN");
    }

    #[test]
    fn validate_still_checks_point_limits() {
        let mut b = WriteBatch::new();
        b.put(vec![0u8; MAX_KEY_LEN + 1], "v");
        assert!(invalid(&b));
        let mut b = WriteBatch::new();
        b.delete(vec![0u8; MAX_KEY_LEN + 1]);
        assert!(invalid(&b));
        let mut b = WriteBatch::new();
        b.put("k", vec![0u8; MAX_VALUE_LEN + 1]);
        assert!(invalid(&b));
        let mut b = WriteBatch::new();
        b.put(vec![0u8; MAX_KEY_LEN], vec![0u8; MAX_VALUE_LEN]).delete(vec![1u8; MAX_KEY_LEN]);
        assert!(b.validate().is_ok());
    }
}
