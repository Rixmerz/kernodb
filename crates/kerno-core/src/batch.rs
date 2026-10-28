//! Atomic write batches and the size limits shared by every engine.

use crate::error::{Error, Result};

/// Upper bound for key length accepted by every candidate.
pub const MAX_KEY_LEN: usize = 512;
/// Upper bound for value length accepted by every candidate.
pub const MAX_VALUE_LEN: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

impl Op {
    pub fn key(&self) -> &[u8] {
        match self {
            Op::Put { key, .. } | Op::Delete { key } => key,
        }
    }
}

/// An atomic group of writes.
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

    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Sum of key and value bytes ("logical bytes" for amplification math).
    pub fn payload_bytes(&self) -> usize {
        self.ops
            .iter()
            .map(|op| match op {
                Op::Put { key, value } => key.len() + value.len(),
                Op::Delete { key } => key.len(),
            })
            .sum()
    }

    /// Checks the size limits shared by every candidate.
    pub fn validate(&self) -> Result<()> {
        for op in &self.ops {
            if op.key().len() > MAX_KEY_LEN {
                return Err(Error::InvalidArgument(format!(
                    "key of {} bytes exceeds MAX_KEY_LEN ({MAX_KEY_LEN})",
                    op.key().len()
                )));
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
