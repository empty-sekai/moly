//! Lookup ownership for resources currently requested by content preparation.
//! Active consumers keep their own handles; eviction only drops the cache's copy.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) struct AssetCache<V, const LIMIT: usize> {
    entries: HashMap<String, (AtomicU64, V)>,
    frame: u64,
}

impl<V, const LIMIT: usize> Default for AssetCache<V, LIMIT> {
    fn default() -> Self {
        assert!(LIMIT > 0);
        Self {
            entries: HashMap::with_capacity(LIMIT),
            frame: 0,
        }
    }
}

impl<V, const LIMIT: usize> AssetCache<V, LIMIT> {
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
    pub(crate) fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.entries.values_mut().map(|(_, value)| value)
    }

    pub(crate) fn get(&self, key: &str) -> Option<&V> {
        let (touched, value) = self.entries.get(key)?;
        touched.store(self.frame, Ordering::Relaxed);
        Some(value)
    }

    pub(crate) fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        let (touched, value) = self.entries.get_mut(key)?;
        touched.store(self.frame, Ordering::Relaxed);
        Some(value)
    }

    pub(crate) fn insert(&mut self, key: String, value: V) {
        // LIMIT is an initial allocation hint, never a cap on live requests.
        // Earlier systems must not evict a request that a later system will
        // touch this frame. Retirement happens once, after all preparation.
        self.entries
            .insert(key, (AtomicU64::new(self.frame), value));
    }

    pub(crate) fn get_or_insert_with(&mut self, key: &str, load: impl FnOnce() -> V) -> &mut V {
        if !self.entries.contains_key(key) {
            self.insert(key.to_owned(), load());
        }
        self.get_mut(key).expect("inserted resource")
    }

    /// Called once after all preparation systems. Keep this frame's requested
    /// resources, drop lookup copies not touched for one complete frame. The
    /// consumer's cloned handles/Arcs are deliberately not inspected or removed.
    pub(crate) fn sweep_unused(&mut self) -> usize {
        let before = self.entries.len();
        self.entries
            .retain(|_, (touched, _)| touched.load(Ordering::Relaxed) == self.frame);
        self.frame = self.frame.checked_add(1).expect("lookup frame exhausted");
        before - self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn browsing_releases_old_ownership_and_keeps_active_consumers_alive() {
        let mut cache = AssetCache::<Arc<()>, 3>::default();
        let playing = Arc::new(());
        cache.insert("playing".into(), playing.clone());
        let unused = Arc::new(());
        let released = Arc::downgrade(&unused);
        cache.insert("unused".into(), unused);
        cache.insert("recent".into(), Arc::new(()));
        cache.sweep_unused();
        cache.get("playing");
        cache.insert("next".into(), Arc::new(()));
        cache.sweep_unused();
        assert!(released.upgrade().is_none());
        assert_eq!(Arc::strong_count(&playing), 2);
        for i in 0..1000 {
            cache.insert(i.to_string(), Arc::new(()));
            cache.sweep_unused();
        }
        assert!(cache.entries.len() <= 3);
        assert_eq!(Arc::strong_count(&playing), 1);
    }

    #[test]
    fn hits_and_replacements_refresh_recency_without_loading_again() {
        let mut cache = AssetCache::<usize, 2>::default();
        cache.insert("a".into(), 1);
        cache.insert("b".into(), 2);
        cache.sweep_unused();
        assert_eq!(*cache.get_or_insert_with("a", || panic!("cache hit")), 1);
        cache.insert("c".into(), 3);
        cache.sweep_unused();
        assert!(cache.get("b").is_none());
        cache.insert("a".into(), 4);
        assert_eq!(cache.entries.len(), 2);
        assert_eq!(cache.get("a"), Some(&4));
    }

    #[test]
    fn idle_sweep_keeps_recent_entries_and_drops_only_cache_ownership() {
        let mut cache = AssetCache::<Arc<()>, 8>::default();
        let recent = Arc::new(());
        let old = Arc::new(());
        let old_weak = Arc::downgrade(&old);
        cache.insert("recent".into(), recent.clone());
        cache.insert("old".into(), old);
        cache.sweep_unused();
        cache.get("recent");
        cache.sweep_unused();
        assert!(cache.get("recent").is_some());
        // `old` is dropped from the cache, but the external `recent` owner is
        // unaffected and remains alive.
        assert!(old_weak.upgrade().is_none());
        assert_eq!(Arc::strong_count(&recent), 2);
    }

    #[test]
    fn simultaneous_preparations_over_soft_limit_do_not_cancel_each_other() {
        let mut cache = AssetCache::<Arc<()>, 2>::default();
        for index in 0..12 {
            cache.insert(index.to_string(), Arc::new(()));
        }
        assert_eq!(cache.len(), 12);
        cache.sweep_unused();
        for index in 0..12 {
            assert!(cache.get(&index.to_string()).is_some());
        }
        cache.sweep_unused();
        assert_eq!(cache.len(), 12);
        cache.sweep_unused();
        assert_eq!(cache.len(), 0);
    }
}
