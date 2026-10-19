//! On-demand catalogue publication for deployment tooling. The exported
//! source-scoped independent availability is the same projection the live UI
//! consumes. No entity, playback, or mutable world access is exposed.
use super::*;

static REQUESTED: AtomicBool = AtomicBool::new(false);
static EXPORT: OnceLock<Mutex<String>> = OnceLock::new();

pub fn library_catalog() -> String {
    let value = EXPORT
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .map(|v| v.clone())
        .unwrap_or_default();
    if value.is_empty() {
        REQUESTED.store(true, Ordering::Relaxed);
        json!({"schemaVersion":SCHEMA,"ready":false}).to_string()
    } else {
        value
    }
}

pub(super) fn publish_if_requested(catalog: &LibraryCatalog) {
    if !REQUESTED.load(Ordering::Relaxed)
        || !catalog.source_ready
        || !catalog.talks_ready
        || !catalog.activities_ready
    {
        return;
    }
    if !REQUESTED.swap(false, Ordering::Relaxed) {
        return;
    }
    let state = ContentLibrary {
        mode: ExperienceMode::Independent,
        ..default()
    };
    let world = LibraryContext::default();
    let entries: Vec<_> = catalog
        .talks
        .iter()
        .map(LibraryTalk::key)
        .chain(catalog.fixtures.iter().map(|f| EntryKey::Fixture(f.id)))
        .chain(
            catalog
                .activities
                .iter()
                .map(|a| EntryKey::Activity(a.spec.key)),
        )
        .map(|key| project_row(key, &state, catalog, &world, true))
        .collect();
    let mut characters: Vec<_> = catalog.character_names.keys().copied().collect();
    characters.sort_unstable();
    let value = json!({"schemaVersion":SCHEMA,"ready":true,"generator":"moly-library",
        "region":catalog.source_region,"version":catalog.source_version,"mode":"independent",
        "characters":characters.into_iter().map(|id| character(catalog,id)).collect::<Vec<_>>(),
        "entries":entries,"issues":catalog.data_issues});
    if let Ok(mut slot) = EXPORT.get_or_init(|| Mutex::new(String::new())).lock() {
        *slot = value.to_string();
    }
}
