//! Language-neutral UI semantics from the actual library definitions.
//! Host renderers translate these codes; they must not interpret Chinese labels
//! or reproduce Rust's eligibility rules.
use super::*;

pub(super) fn enrich(
    key: EntryKey,
    state: &ContentLibrary,
    catalog: &LibraryCatalog,
    row: &mut Value,
) {
    if let Some(source) = catalog.talk(key) {
        row["preview"] = match &source.preview_tweet {
            Some(tweet) => json!({"available":true,"tweetId":tweet.id,"text":tweet.text,"unit":source.units.first()}),
            None => json!({"available":false,"reason":"missing_source_tweet"}),
        };
    }
    let (category, behavior, action, text_mode, fixtures, units) = match key {
        EntryKey::Fixture(id) => {
            let source = catalog.fixture(id);
            let behavior = source
                .map(|f| match &f.presentation {
                    FixturePresentation::Surface { wall: true, .. } => "wall",
                    FixturePresentation::Surface { wall: false, .. } => "floor",
                    FixturePresentation::Custom { .. } => "custom",
                    _ => match f.action.as_str() {
                        "timeline" => "timeline",
                        "loop" => "loop",
                        "one_shot" => "one_shot",
                        _ => "static",
                    },
                })
                .unwrap_or("static");
            (
                "furniture",
                behavior,
                if source.is_some_and(LibraryFixture::interactive) {
                    "play"
                } else {
                    "inspect"
                },
                "none",
                vec![id],
                vec![],
            )
        }
        EntryKey::Activity(id) => {
            let source = catalog.activity(id);
            let bubble = source.is_some_and(|a| a.spec.tweet.is_some());
            let fixtures = source.map(|a| vec![a.spec.fixture_id]).unwrap_or_default();
            let units = source.map(|a| vec![a.spec.unit]).unwrap_or_default();
            if let Some(source) = source {
                row["variant"] = json!({"index":source.spec.variant,"count":source.spec.variants});
                // These are original names separated by typography, not a
                // Chinese UI template disguised as authored content.
                row["title"] = json!(format!(
                    "{} · {}",
                    catalog.character(source.spec.unit),
                    catalog.fixture_name(source.spec.fixture_id)
                ));
                if !bubble {
                    row["subtitle"] = json!("");
                }
            }
            row["description"] = json!("");
            (
                "activity",
                "authored",
                "play",
                if bubble { "bubble" } else { "none" },
                fixtures,
                units,
            )
        }
        _ => {
            let source = catalog.talk(key);
            let category = source
                .map(|t| {
                    if t.drives_fixture {
                        "fixture_performance"
                    } else if t.furniture_related {
                        "fixture_story"
                    } else {
                        "conversation"
                    }
                })
                .unwrap_or("conversation");
            let text_mode = if source.is_some_and(|t| !t.lines.is_empty()) {
                "transcript"
            } else {
                "none"
            };
            let fixtures = source.map(|t| t.fixture_ids.clone()).unwrap_or_default();
            let units = source.map(|t| t.units.clone()).unwrap_or_default();
            if text_mode == "none" && !fixtures.is_empty() {
                row["title"] = json!(fixtures
                    .iter()
                    .map(|id| catalog.fixture_name(*id))
                    .collect::<Vec<_>>()
                    .join(" · "));
                row["subtitle"] = json!("");
            }
            (category, "authored", "play", text_mode, fixtures, units)
        }
    };
    row["presentation"] = json!({"category":category,"behavior":behavior,"primaryAction":action,"textMode":text_mode});
    row["fixtureIds"] = json!(fixtures);
    row["unitIds"] = json!(units);
    row["reasonCode"] = if row["available"].as_bool() == Some(false) {
        json!(if state.mode == ExperienceMode::Independent {
            "source_unavailable"
        } else {
            "scene_unavailable"
        })
    } else {
        Value::Null
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn static_furniture_is_not_a_play_action() {
        let mut row = json!({"available":false});
        enrich(
            EntryKey::Fixture(7),
            &ContentLibrary::default(),
            &LibraryCatalog::default(),
            &mut row,
        );
        assert_eq!(row["presentation"]["primaryAction"], "inspect");
        assert_eq!(row["presentation"]["textMode"], "none");
        assert_eq!(row["reasonCode"], "source_unavailable");
        assert_eq!(row["fixtureIds"], json!([7]));
    }
}
