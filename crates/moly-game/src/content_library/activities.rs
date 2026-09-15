//! Read-only product projection of the existing, source-qualified NPC activity
//! catalogue. It exposes authored variants and bubbles without inventing a talk.
use super::*;
use crate::fixture_activity_data::{
    ActivityKey, ActivityOrigin, ActivitySpec, FixtureActivityTables,
};

#[derive(Clone)]
pub(super) struct LibraryActivity {
    pub spec: ActivitySpec,
    pub title: String,
    pub search: String,
    pub unavailable: Option<String>,
}
impl LibraryActivity {
    pub fn kind(&self) -> &'static str {
        match self.spec.key.origin {
            ActivityOrigin::NoTalk(_) => "无对白动作",
            ActivityOrigin::PreAction(_) if self.spec.tweet.is_some() => "动作与头顶气泡",
            ActivityOrigin::PreAction(_) => "家具前置演出",
        }
    }
}

pub(crate) fn build_activity_catalog(
    mut catalog: ResMut<LibraryCatalog>,
    tables: Option<Res<FixtureActivityTables>>,
    scripts: Option<Res<TalkStore>>,
    capabilities: Res<capabilities::ActivityCapabilities>,
) {
    if catalog.activities_ready
        || !catalog.source_ready
        || !catalog.talks_ready
        || !capabilities.ready()
    {
        return;
    }
    let (Some(tables), Some(scripts)) = (tables, scripts) else {
        return;
    };
    let rows = tables.catalogue_activities(&scripts);
    catalog.activities = rows
        .into_iter()
        .map(|spec| {
            let character = catalog.character(spec.unit);
            let fixture = catalog.fixture_name(spec.fixture_id);
            let title = if spec.variants > 1 {
                format!("{character} · {fixture} · 动作 {}", spec.variant)
            } else {
                format!("{character} · {fixture}")
            };
            let search = format!(
                "{character} {fixture} {} {} {} {} {}",
                spec.key.source_id(),
                spec.fixture_id,
                spec.timeline.id,
                spec.tweet
                    .as_ref()
                    .map(|tweet| tweet.text.as_str())
                    .unwrap_or(""),
                if spec.tweet.is_some() {
                    "头顶气泡 前置演出"
                } else {
                    "无对白动作"
                }
            )
            .to_lowercase();
            let unavailable = capabilities.reason(&spec.timeline.asset_name);
            LibraryActivity {
                spec,
                title,
                search,
                unavailable,
            }
        })
        .collect();
    catalog.activities_ready = true;
    catalog.revision = catalog.revision.wrapping_add(1);
    info!(
        "[content-library] indexed {} source-qualified character/furniture activities",
        catalog.activities.len()
    );
}

pub(super) fn filtered_keys(
    state: &ContentLibrary,
    catalog: &LibraryCatalog,
    context: &LibraryContext,
) -> Vec<EntryKey> {
    let query = state.search.trim().to_lowercase();
    catalog
        .activities
        .iter()
        .filter(|row| {
            let spec = &row.spec;
            let matches = if let Ok(number) = query.trim_start_matches('#').parse::<i32>() {
                number == spec.fixture_id
                    || number == spec.key.source_id()
                    || number == spec.timeline.id
                    || spec.related_talk == Some(number)
            } else {
                query
                    .split_whitespace()
                    .all(|part| row.search.contains(part))
            };
            matches
                && state.character.is_none_or(|unit| unit == spec.unit)
                && state.related_fixture.is_none_or(|id| id == spec.fixture_id)
                && match state.scope {
                    Scope::All => true,
                    Scope::Here => current_reason(row, catalog, context).is_none(),
                    Scope::Ready if state.mode == ExperienceMode::Independent => {
                        context::independent_reason(EntryKey::Activity(spec.key), catalog).is_none()
                    }
                    Scope::Ready => current_reason(row, catalog, context).is_none(),
                }
        })
        .map(|row| EntryKey::Activity(row.spec.key))
        .collect()
}

pub(super) fn current_reason(
    row: &LibraryActivity,
    catalog: &LibraryCatalog,
    context: &LibraryContext,
) -> Option<String> {
    if row.unavailable.is_some() {
        return row.unavailable.clone();
    }
    if !context.actors.contains_key(&row.spec.unit) {
        return Some(format!(
            "需要 {} 来到当前场景",
            catalog.character(row.spec.unit)
        ));
    }
    if !context
        .instances
        .get(&row.spec.fixture_id)
        .is_some_and(|rows| !rows.is_empty())
    {
        return Some(format!(
            "需要在当前场景摆放 {}",
            catalog.fixture_name(row.spec.fixture_id)
        ));
    }
    None
}

impl LibraryCatalog {
    pub(super) fn activity(&self, key: ActivityKey) -> Option<&LibraryActivity> {
        self.activities.iter().find(|row| row.spec.key == key)
    }
}
