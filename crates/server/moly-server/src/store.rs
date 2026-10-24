//! Where a user's document is kept between runs.
//!
//! The host installs the store: a file store below `MOLY_SERVER_DATA` when it
//! names a directory, else an in-memory store, so a harness run starts from
//! its own state every time. The server asks for a user's document when it
//! first hears from that user and writes it back when a call changed it.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use bevy::prelude::*;
use moly_protocol::UserId;

/// One user's stored document.
pub trait DocumentStore: Send + Sync + 'static {
    /// The user's document, or `None` when the user has none yet.
    fn load(&self, user: &UserId) -> Option<String>;
    /// Stores the user's document. A store that cannot write names the
    /// failure in the server's log; the call that changed the document is
    /// still answered.
    fn save(&self, user: &UserId, text: &str);
    /// Where the user's document is kept, when it is a file.
    fn location(&self, _user: &UserId) -> Option<PathBuf> {
        None
    }
}

/// The installed store.
#[derive(Resource)]
pub struct Store(pub Box<dyn DocumentStore>);

impl Store {
    /// A store that forgets on exit.
    pub fn memory() -> Self {
        Self(Box::new(MemoryStore::default()))
    }

    /// A store that keeps `<directory>/<user>.json`.
    pub fn file(directory: PathBuf) -> Self {
        Self(Box::new(FileStore { directory }))
    }
}

/// The store a server without `MOLY_SERVER_DATA` runs with.
#[derive(Default)]
struct MemoryStore(Mutex<BTreeMap<UserId, String>>);

impl DocumentStore for MemoryStore {
    fn load(&self, user: &UserId) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(user)
            .cloned()
    }

    fn save(&self, user: &UserId, text: &str) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(user.clone(), text.to_owned());
    }
}

/// `<directory>/<user>.json` per user.
struct FileStore {
    directory: PathBuf,
}

impl FileStore {
    fn path(&self, user: &UserId) -> PathBuf {
        // The hub admits only valid ids; a path is never built from another.
        assert!(user.is_valid(), "a document path for the invalid user id {user:?}");
        self.directory.join(format!("{user}.json"))
    }
}

impl DocumentStore for FileStore {
    fn location(&self, user: &UserId) -> Option<PathBuf> {
        Some(self.path(user))
    }

    fn load(&self, user: &UserId) -> Option<String> {
        match std::fs::read_to_string(self.path(user)) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                warn!(
                    "[server] user {user}: the stored document {} is unreadable: {error}",
                    self.path(user).display()
                );
                None
            }
        }
    }

    fn save(&self, user: &UserId, text: &str) {
        let path = self.path(user);
        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                warn!(
                    "[server] user {user}: the document directory {} cannot be made: {error}",
                    parent.display()
                );
                return;
            }
        }
        if let Err(error) = std::fs::write(&path, text) {
            warn!(
                "[server] user {user}: the document {} is not written: {error}",
                path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_memory_store_keeps_one_document_per_user() {
        let store = MemoryStore::default();
        let (a, b) = (UserId("a".into()), UserId("b".into()));
        assert_eq!(store.load(&a), None);
        store.save(&a, "one");
        assert_eq!(store.load(&a).as_deref(), Some("one"));
        assert_eq!(store.load(&b), None);
    }

    #[test]
    fn the_file_store_writes_and_reads_a_user_document() {
        let directory = std::env::temp_dir().join(format!(
            "moly-server-store-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let store = FileStore {
            directory: directory.clone(),
        };
        let user = UserId::local();
        assert_eq!(store.load(&user), None);
        store.save(&user, "{\"schemaVersion\":2}");
        assert_eq!(store.load(&user).as_deref(), Some("{\"schemaVersion\":2}"));
        assert!(directory.join("local.json").is_file());
        assert_eq!(store.location(&user), Some(directory.join("local.json")));
        let _ = std::fs::remove_dir_all(&directory);
    }
}
