//! Where backups live: a flat namespace of objects named by `/`-separated keys, as on S3 and R2.
//!
//! Three stores implement it: [`MemStore`] (tests), [`FsStore`] (a directory: a mounted volume, or
//! an NFS share) and [`crate::s3::S3Store`]. [`Prefixed`] gives one tenant its own corner of a store.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::error::{Error, Result};

pub trait ObjectStore: Send + Sync {
    /// Stores `data` under `key`, replacing an object that is there. A reader sees the old object
    /// or the whole new one, never a mix.
    fn put(&self, key: &str, data: &[u8]) -> Result<()>;
    /// The object under `key`, `None` when there is none.
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    /// Every key that starts with `prefix`, in byte order.
    fn list(&self, prefix: &str) -> Result<Vec<String>>;
    /// Removes `key`; removing a key that is not there is not an error.
    fn delete(&self, key: &str) -> Result<()>;
}

/// Keys are made of what a file name and an S3 key both allow, so no store has to escape anything.
pub(crate) fn check_key(key: &str) -> Result<()> {
    let ok = !key.is_empty()
        && key.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        });
    if ok { Ok(()) } else { Err(Error::Store(format!("{key:?} is not a valid object key"))) }
}

/// Objects in memory.
#[derive(Default)]
pub struct MemStore {
    objects: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl MemStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Vec<u8>>> {
        self.objects.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ObjectStore for MemStore {
    fn put(&self, key: &str, data: &[u8]) -> Result<()> {
        check_key(key)?;
        self.lock().insert(key.to_owned(), data.to_vec());
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        check_key(key)?;
        Ok(self.lock().get(key).cloned())
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        Ok(self.lock().keys().filter(|k| k.starts_with(prefix)).cloned().collect())
    }

    fn delete(&self, key: &str) -> Result<()> {
        check_key(key)?;
        self.lock().remove(key);
        Ok(())
    }
}

/// Objects as files under a directory.
pub struct FsStore {
    root: PathBuf,
}

/// Marks a file that is still being written; `list` does not show it.
const TMP: &str = ".tmp.";
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl FsStore {
    /// A store in `root`, created on the first `put`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        FsStore { root: root.into() }
    }

    fn path(&self, key: &str) -> Result<PathBuf> {
        check_key(key)?;
        Ok(self.root.join(key))
    }

    fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) -> Result<()> {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let key = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            if entry.file_type()?.is_dir() {
                Self::walk(&entry.path(), &key, out)?;
            } else if !name.contains(TMP) {
                out.push(key);
            }
        }
        Ok(())
    }
}

impl ObjectStore for FsStore {
    fn put(&self, key: &str, data: &[u8]) -> Result<()> {
        let path = self.path(key)?;
        let parent = path.parent().unwrap_or(&self.root);
        std::fs::create_dir_all(parent)?;
        let tmp = parent.join(format!(
            "{}{TMP}{}.{}",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("object"),
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let written = (|| -> std::io::Result<()> {
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(data)?;
            // The object is durable before it is named: a crash leaves no short object.
            file.sync_all()?;
            std::fs::rename(&tmp, &path)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        written.map_err(|e| Error::Store(format!("cannot write {key}: {e}")))
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        match std::fs::read(self.path(key)?) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Store(format!("cannot read {key}: {e}"))),
        }
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let mut keys = Vec::new();
        Self::walk(&self.root, "", &mut keys)?;
        keys.retain(|k| k.starts_with(prefix));
        keys.sort();
        Ok(keys)
    }

    fn delete(&self, key: &str) -> Result<()> {
        match std::fs::remove_file(self.path(key)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Store(format!("cannot delete {key}: {e}"))),
        }
    }
}

/// A store seen through a key prefix: `Prefixed::new(store, "acme")` keeps what it stores under
/// `acme/` and shows nothing outside it.
pub struct Prefixed {
    inner: Arc<dyn ObjectStore>,
    prefix: String,
}

impl Prefixed {
    pub fn new(inner: Arc<dyn ObjectStore>, prefix: &str) -> Self {
        let prefix = prefix.trim_matches('/');
        Prefixed { inner, prefix: if prefix.is_empty() { String::new() } else { format!("{prefix}/") } }
    }

    fn key(&self, key: &str) -> String {
        format!("{}{key}", self.prefix)
    }
}

impl ObjectStore for Prefixed {
    fn put(&self, key: &str, data: &[u8]) -> Result<()> {
        self.inner.put(&self.key(key), data)
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        self.inner.get(&self.key(key))
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let all = self.inner.list(&self.key(prefix))?;
        Ok(all.into_iter().filter_map(|k| k.strip_prefix(&self.prefix).map(str::to_owned)).collect())
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.inner.delete(&self.key(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exercise(store: &dyn ObjectStore) {
        assert_eq!(store.get("a/b").unwrap(), None);
        store.put("a/b", b"one").unwrap();
        store.put("a/b", b"two").unwrap();
        store.put("a/c.x", b"3").unwrap();
        store.put("z", b"4").unwrap();
        assert_eq!(store.get("a/b").unwrap().as_deref(), Some(&b"two"[..]));
        assert_eq!(store.list("a/").unwrap(), ["a/b", "a/c.x"]);
        assert_eq!(store.list("").unwrap(), ["a/b", "a/c.x", "z"]);
        assert_eq!(store.list("q").unwrap(), Vec::<String>::new());
        store.delete("a/b").unwrap();
        store.delete("a/b").unwrap();
        assert_eq!(store.get("a/b").unwrap(), None);
        for bad in ["", "/a", "a//b", "a/../b", "a b", "a/"] {
            assert!(store.put(bad, b"x").is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_memory_store_behaves() {
        exercise(&MemStore::default());
    }

    #[test]
    fn the_file_store_behaves_and_hides_files_being_written() {
        let dir = std::env::temp_dir().join(format!("kb-fsstore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = FsStore::new(&dir);
        exercise(&store);
        std::fs::write(dir.join("a").join(format!("half{TMP}1.1")), b"x").unwrap();
        assert_eq!(store.list("a/").unwrap(), ["a/c.x"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_prefixed_store_stays_inside_its_prefix() {
        let inner: Arc<dyn ObjectStore> = Arc::new(MemStore::default());
        let (a, b) = (Prefixed::new(Arc::clone(&inner), "a"), Prefixed::new(Arc::clone(&inner), "/b/"));
        a.put("k", b"A").unwrap();
        b.put("k", b"B").unwrap();
        assert_eq!(a.get("k").unwrap().as_deref(), Some(&b"A"[..]));
        assert_eq!(a.list("").unwrap(), ["k"]);
        assert_eq!(inner.list("").unwrap(), ["a/k", "b/k"]);
        a.delete("k").unwrap();
        assert_eq!(b.get("k").unwrap().as_deref(), Some(&b"B"[..]));
        assert_eq!(Prefixed::new(inner, "").key("x"), "x");
    }
}
