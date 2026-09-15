//! Source-backed catalog projection. It never chooses a random playback row.
use super::*;
use serde_json::Value;

pub(crate) fn parse_assets(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    handles: Option<ResMut<LibraryAssets>>,
    mut catalog: ResMut<LibraryCatalog>,
) {
    let Some(mut handles) = handles else {
        return;
    };
    let sources = [
        (0, handles.characters.clone()),
        (1, handles.fixtures.clone()),
        (2, handles.thumbnails.clone()),
        (3, handles.models.clone()),
    ];
    for (slot, handle) in sources {
        if handles.processed[slot] {
            continue;
        }
        if slot == 3 && !handles.processed[1] {
            continue;
        }
        if let LoadState::Failed(error) = server.load_state(&handle) {
            warn!("[content-library] asset slot {slot} failed: {error:?}");
            catalog.data_issues.push(
                match slot {
                    0 => "角色名称暂时无法载入，部分条目使用备用名称",
                    1 => "家具图鉴暂时无法载入，对话仍可浏览",
                    2 => "部分家具图片暂时无法载入",
                    _ => "家具模型清单暂时无法载入，仍可浏览图鉴",
                }
                .into(),
            );
            handles.processed[slot] = true;
            continue;
        }
        let Some(asset) = jsons.get(&handle) else {
            continue;
        };
        match serde_json::from_str::<Value>(&asset.0) {
            Ok(value) => match slot {
                0 => parse_characters(&value, &mut catalog),
                1 => parse_fixtures(&value, &mut catalog),
                2 => parse_thumbnails(&value, &mut catalog),
                _ => parse_model_index(&value, &mut catalog),
            },
            Err(error) => {
                warn!("[content-library] invalid document slot {slot}: {error}");
                catalog
                    .data_issues
                    .push("一份展示数据格式异常，其余内容仍可使用".into());
            }
        }
        handles.processed[slot] = true;
    }
    if handles.processed.iter().all(|done| *done) {
        let paths = catalog.thumbnail_paths.clone();
        for row in &mut catalog.fixtures {
            row.thumbnail = paths.get(&row.id).map(|path| {
                server.load::<Image>(AssetPath::from(format!("moly://fixture-thumbnails/{path}")))
            });
        }
        catalog.fixtures.sort_by_key(|row| row.id);
        catalog.source_ready = true;
        catalog.revision = catalog.revision.wrapping_add(1);
        commands.remove_resource::<LibraryAssets>();
    }
}

fn positive_id(value: Option<&Value>) -> Option<i32> {
    value?
        .as_i64()
        .and_then(|id| i32::try_from(id).ok())
        .filter(|id| *id > 0)
}
fn parse_characters(value: &Value, catalog: &mut LibraryCatalog) {
    let Some(rows) = value.get("characters").and_then(Value::as_object) else {
        catalog.data_issues.push("角色展示数据缺少名册".into());
        return;
    };
    for (key, row) in rows {
        let Ok(unit) = key.parse::<u32>() else {
            continue;
        };
        if let Some(name) = row
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
        {
            catalog.character_names.insert(unit, plain_text(name));
        }
        if let Some(color) = row.pointer("/identity/colorCode").and_then(Value::as_str) {
            catalog.character_colors.insert(unit, color.to_owned());
        }
    }
}
fn parse_fixtures(value: &Value, catalog: &mut LibraryCatalog) {
    catalog.source_region = value
        .get("region")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    catalog.source_version = value
        .get("gameVersion")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let Some(rows) = value.get("fixtures").and_then(Value::as_array) else {
        catalog.data_issues.push("家具展示数据缺少条目列表".into());
        return;
    };
    let mut seen = HashSet::new();
    for (index, row) in rows.iter().enumerate() {
        let Some(id) = positive_id(row.get("id")).filter(|id| seen.insert(*id)) else {
            catalog.data_issues.push(format!(
                "第 {} 条家具数据缺少有效编号或与其他条目重复",
                index + 1
            ));
            continue;
        };
        let name = row
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .map(plain_text)
            .unwrap_or_else(|| format!("未命名家具 · {id}"));
        let description = row
            .get("description")
            .and_then(Value::as_str)
            .map(plain_text)
            .unwrap_or_default();
        let action = row
            .get("playerActionType")
            .and_then(Value::as_str)
            .unwrap_or("no_action")
            .to_owned();
        let preview = &row["preview"];
        let custom = (preview["kind"].as_str() == Some("custom"))
            .then(|| preview["variants"].as_array())
            .flatten()
            .and_then(|variants| {
                variants
                    .iter()
                    .find(|value| value["available"].as_bool() == Some(true))
            });
        let custom_base = custom
            .and_then(|value| value["basePackage"].as_str())
            .and_then(|name| name.strip_prefix("mysekai__fixture__"))
            .and_then(source_leaf);
        let model = row
            .get("assetbundleName")
            .and_then(Value::as_str)
            .and_then(source_leaf);
        let model = custom_base.or(model);
        let presentation = if preview["kind"].as_str() == Some("surface") {
            match (preview["skin"].as_str(), preview["channel"].as_str()) {
                (Some(skin), Some(channel @ ("wall" | "floor")))
                    if skin
                        .bytes()
                        .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'-')
                        && !skin.is_empty() =>
                {
                    FixturePresentation::Surface {
                        skin: skin.into(),
                        wall: channel == "wall",
                        available: preview["available"].as_bool() == Some(true),
                    }
                }
                _ => FixturePresentation::default(),
            }
        } else if let Some(path) = custom
            .and_then(|value| value["ornamentFile"].as_str())
            .filter(|path| {
                path.starts_with("custom-fixture-models/")
                    && path.ends_with(".glb")
                    && !path.contains([':', '\\'])
                    && !path.split('/').any(|part| matches!(part, "" | ".." | "."))
            })
        {
            FixturePresentation::Custom {
                ornament: path.into(),
            }
        } else {
            FixturePresentation::default()
        };
        let dimensions = ["gridWidth", "gridHeight", "gridDepth"].map(|field| {
            row.get(field)
                .and_then(Value::as_i64)
                .and_then(|value| i32::try_from(value).ok())
        });
        let dimensions = if let Some(custom) = custom {
            ["width", "height", "depth"].map(|axis| {
                custom["gridSize"][axis]
                    .as_i64()
                    .and_then(|v| i32::try_from(v).ok())
            })
        } else {
            dimensions
        };
        let source = model
            .zip(dimensions.into_iter().collect::<Option<Vec<_>>>())
            .and_then(|(model, size)| {
                (size.iter().all(|value| *value > 0)).then(|| FixtureSource {
                    package: format!("mysekai__fixture__{model}"),
                    grid_size: moly_law::fixture::Vector3Int::new(size[0], size[1], size[2]),
                    exported: false,
                    layout: match row
                        .get("layoutType")
                        .and_then(Value::as_str)
                        .or_else(|| row.get("handleType").and_then(Value::as_str))
                    {
                        Some("wall" | "windowpane" | "clock") => {
                            moly_law::fixture::position::layout_type::WALL_FRONT
                        }
                        Some("road") => moly_law::fixture::position::layout_type::ROAD,
                        _ if model.contains("_rug_") => {
                            moly_law::fixture::position::layout_type::RUG
                        }
                        _ => moly_law::fixture::position::layout_type::FLOOR,
                    },
                    center_y: if matches!(
                        row.get("layoutType")
                            .and_then(Value::as_str)
                            .or_else(|| row.get("handleType").and_then(Value::as_str)),
                        Some("wall" | "windowpane" | "clock")
                    ) {
                        6
                    } else {
                        0
                    },
                })
            });
        let search = format!("{name} {description} #{id}").to_lowercase();
        catalog.fixtures.push(LibraryFixture {
            id,
            name,
            description,
            action,
            search,
            thumbnail: None,
            source,
            presentation,
        });
    }
}
fn source_leaf(value: &str) -> Option<&str> {
    let leaf = value
        .rsplit(['/', '\\'])
        .next()?
        .strip_suffix(".prefab")
        .unwrap_or_else(|| value.rsplit(['/', '\\']).next().unwrap_or(value));
    (!leaf.is_empty() && !leaf.contains([':', '/', '\\'])).then_some(leaf)
}
fn parse_model_index(value: &Value, catalog: &mut LibraryCatalog) {
    let Some(packages) = value.get("packages").and_then(Value::as_object) else {
        catalog.data_issues.push("家具模型清单缺少包索引".into());
        return;
    };
    for row in &mut catalog.fixtures {
        let Some(source) = &mut row.source else {
            continue;
        };
        let Some(package) = packages.get(&source.package) else {
            continue;
        };
        let glb = package
            .get("glb")
            .and_then(Value::as_str)
            .unwrap_or_default();
        source.exported = package.get("status").and_then(Value::as_str) == Some("exported")
            && package.get("hasFixtureView").and_then(Value::as_bool) == Some(true)
            && glb == format!("{}.glb", source.package);
    }
}
fn safe_image_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains([':', '\\'])
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
        && path.to_ascii_lowercase().ends_with(".png")
}
fn parse_thumbnails(value: &Value, catalog: &mut LibraryCatalog) {
    let Some(rows) = value.get("fixtures").and_then(Value::as_array) else {
        catalog
            .data_issues
            .push("家具图片清单格式异常，仍可按名称浏览".into());
        return;
    };
    for row in rows {
        if let (Some(id), Some(path)) = (
            positive_id(row.get("fixtureId")),
            row.pointer("/variants/0/image").and_then(Value::as_str),
        ) {
            if safe_image_path(path) {
                catalog
                    .thumbnail_paths
                    .entry(id)
                    .or_insert_with(|| path.to_owned());
            } else {
                warn!("[content-library] ignored invalid thumbnail path for fixture {id}");
            }
        }
    }
}

pub(crate) fn build_talk_catalog(
    store: Option<Res<PlayerTalkStore>>,
    fixtures: Option<Res<TalkStore>>,
    mut catalog: ResMut<LibraryCatalog>,
) {
    if catalog.talks_ready || !catalog.source_ready {
        return;
    }
    let (Some(store), Some(fixtures)) = (store, fixtures) else {
        return;
    };
    let mut talks = Vec::new();
    let mut identities = HashSet::new();
    let mut units: Vec<_> = store.units.iter().collect();
    units.sort_by_key(|(unit, _)| *unit);
    for (unit, rows) in units {
        for row in rows {
            if !identities.insert((0, row.talk_id)) {
                catalog
                    .data_issues
                    .push(format!("对话 {} 的普通脚本重复，已保留第一份", row.talk_id));
                continue;
            }
            let fixture_ids: Vec<i32> = row
                .conditions
                .iter()
                .zip(&row.condition_values)
                .filter_map(|(kind, value)| {
                    (condition_type_discriminant(kind) == Some(CONDITION_MYSEKAI_FIXTURE_ID))
                        .then_some(*value)
                        .flatten()
                })
                .filter(|id| *id > 0)
                .collect();
            let related = row.conditions.iter().any(|kind| {
                matches!(
                    condition_type_discriminant(kind),
                    Some(
                        CONDITION_MYSEKAI_FIXTURE_ID
                            | CONDITION_MYSEKAI_FIXTURE_TAG_ID
                            | CONDITION_AFTER_SET_FIXTURE
                    )
                )
            });
            let lines = general_lines(&row.steps, &catalog.character(*unit), &row.tweet.text);
            talks.push(make_talk(
                TalkContent {
                    master_id: row.talk_id,
                    backend: TalkBackend::General,
                    is_general: Some(crate::player_talk::is_general_row(row)),
                },
                vec![*unit],
                fixture_ids,
                lines,
                related,
                false,
                &catalog,
            ));
        }
    }
    for row in &fixtures.rows {
        if !identities.insert((1, row.talk_id)) {
            catalog
                .data_issues
                .push(format!("对话 {} 的家具脚本重复，已保留第一份", row.talk_id));
            continue;
        }
        let mut fixture_ids = row.fixture_ids.clone();
        fixture_ids.extend(row.pairs.iter().map(|pair| pair.fixture_id));
        fixture_ids.extend(row.steps.iter().filter_map(fixture_step_id));
        let mut seen = HashSet::new();
        fixture_ids.retain(|id| *id > 0 && seen.insert(*id));
        let units: Vec<u32> = row
            .unit_ids
            .iter()
            .filter_map(|unit| u32::try_from(*unit).ok())
            .filter(|unit| *unit > 0)
            .collect();
        let fallback = units
            .first()
            .map(|unit| catalog.character(*unit))
            .unwrap_or_else(|| "旁白".into());
        let lines = fixture_lines(&row.steps, &fallback, &row.tweet.text);
        let drives = row.steps.iter().any(is_fixture_operation);
        // A backend filename is not a semantic category. Actual bindings and
        // operations, not the fixture-script representation alone, imply it.
        let related = !fixture_ids.is_empty() || drives;
        talks.push(make_talk(
            TalkContent {
                master_id: row.talk_id,
                backend: TalkBackend::Fixture,
                is_general: None,
            },
            units,
            fixture_ids,
            lines,
            related,
            drives,
            &catalog,
        ));
    }
    catalog.talks = talks;
    catalog.talks_ready = true;
    catalog.revision = catalog.revision.wrapping_add(1);
    info!(
        "[content-library] indexed {} talks and {} furniture definitions",
        catalog.talks.len(),
        catalog.fixtures.len()
    );
}
fn make_talk(
    content: TalkContent,
    units: Vec<u32>,
    fixture_ids: Vec<i32>,
    lines: Vec<DialogueLine>,
    furniture_related: bool,
    drives_fixture: bool,
    catalog: &LibraryCatalog,
) -> LibraryTalk {
    let preview = lines
        .iter()
        .map(|line| format!("{}\n{}", line.speaker, line.text))
        .collect::<Vec<_>>()
        .join("\n\n");
    let title = lines
        .first()
        .map(|line| excerpt(&line.text, 44))
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| {
            if drives_fixture {
                "一段家具演出".into()
            } else {
                "一段日常对话".into()
            }
        });
    let cast = units
        .iter()
        .map(|unit| catalog.character(*unit))
        .collect::<Vec<_>>()
        .join(" ");
    let furniture = fixture_ids
        .iter()
        .map(|id| catalog.fixture_name(*id))
        .collect::<Vec<_>>()
        .join(" ");
    let search = format!(
        "#{id} {title} {preview} {cast} {furniture}",
        id = content.master_id
    )
    .to_lowercase();
    LibraryTalk {
        content,
        units,
        fixture_ids,
        title,
        preview,
        lines,
        search,
        furniture_related,
        drives_fixture,
    }
}
fn general_lines(steps: &[TalkStep], fallback: &str, tweet: &str) -> Vec<DialogueLine> {
    let mut speaker = fallback.to_owned();
    let mut lines = Vec::new();
    for step in steps {
        match step {
            TalkStep::Label { name } if !name.is_empty() => speaker = plain_text(name),
            TalkStep::Text { text } if !text.is_empty() => lines.push(DialogueLine {
                speaker: speaker.clone(),
                text: plain_text(text),
            }),
            _ => {}
        }
    }
    if lines.is_empty() && !tweet.is_empty() {
        lines.push(DialogueLine {
            speaker,
            text: plain_text(tweet),
        });
    }
    lines
}
fn fixture_lines(steps: &[FixtureStep], fallback: &str, tweet: &str) -> Vec<DialogueLine> {
    let mut speaker = fallback.to_owned();
    let mut lines = Vec::new();
    for step in steps {
        match step {
            FixtureStep::Label { name } if !name.is_empty() => speaker = plain_text(name),
            FixtureStep::Text { text } if !text.is_empty() => lines.push(DialogueLine {
                speaker: speaker.clone(),
                text: plain_text(text),
            }),
            _ => {}
        }
    }
    if lines.is_empty() && !tweet.is_empty() {
        lines.push(DialogueLine {
            speaker,
            text: plain_text(tweet),
        });
    }
    lines
}
fn fixture_step_id(step: &FixtureStep) -> Option<i32> {
    let value = match step {
        FixtureStep::LookAtToNpc { fixture, .. }
        | FixtureStep::FixtureVoice { fixture, .. }
        | FixtureStep::ChangeFixtureCharacterEye { fixture, .. }
        | FixtureStep::ChangeFixtureCharacterMouth { fixture, .. }
        | FixtureStep::ChangeFixtureTimeline { fixture, .. }
        | FixtureStep::ShowFixtureEmoticon { fixture, .. } => *fixture,
        // LookAtFixture.fixture is an actor referent despite its extracted key.
        _ => return None,
    };
    (value.is_finite() && value.fract() == 0. && value > 0. && value <= i32::MAX as f64)
        .then_some(value as i32)
}
fn is_fixture_operation(step: &FixtureStep) -> bool {
    matches!(
        step,
        FixtureStep::FixtureVoice { .. }
            | FixtureStep::ChangeFixtureCharacterEye { .. }
            | FixtureStep::ChangeFixtureCharacterMouth { .. }
            | FixtureStep::ChangeFixtureTimeline { .. }
            | FixtureStep::ShowFixtureEmoticon { .. }
            | FixtureStep::PlayFixtureGimmick { .. }
            | FixtureStep::StopFixtureGimmick { .. }
    )
}
fn query_matches(haystack: &str, query: &str, id: i32) -> bool {
    let query = query.trim();
    if let Ok(number) = query.trim_start_matches('#').parse::<i32>() {
        return id == number;
    }
    query.split_whitespace().all(|word| haystack.contains(word))
}
pub(super) fn filtered_keys(
    state: &ContentLibrary,
    catalog: &LibraryCatalog,
    world: &LibraryContext,
) -> Vec<EntryKey> {
    let query = state.search.to_lowercase();
    let mut keys = match state.tab {
        LibraryTab::Activities => activities::filtered_keys(state, catalog, world),
        LibraryTab::Furniture => catalog
            .fixtures
            .iter()
            .filter(|row| {
                query_matches(&row.search, &query, row.id)
                    && (!state.special_only || row.interactive())
                    && match state.scope {
                        Scope::All => true,
                        Scope::Here => world
                            .instances
                            .get(&row.id)
                            .is_some_and(|items| !items.is_empty()),
                        Scope::Ready => {
                            if state.mode == ExperienceMode::Independent {
                                row.source.as_ref().is_some_and(|source| source.exported)
                            } else {
                                row.interactive()
                                    && world
                                        .instances
                                        .get(&row.id)
                                        .is_some_and(|items| items.iter().any(|item| item.ready))
                            }
                        }
                    }
            })
            .map(|row| EntryKey::Fixture(row.id))
            .collect::<Vec<_>>(),
        _ => catalog
            .talks
            .iter()
            .filter(|row| {
                (state.tab != LibraryTab::Performances || row.furniture_related)
                    && (!state.special_only || row.drives_fixture)
                    && state.character.is_none_or(|unit| row.units.contains(&unit))
                    && state
                        .related_fixture
                        .is_none_or(|id| row.fixture_ids.contains(&id))
                    && query_matches(&row.search, &query, row.content.master_id)
                    && match state.scope {
                        Scope::All => true,
                        Scope::Here => context::talk_here(row, world),
                        Scope::Ready => {
                            if state.mode == ExperienceMode::Independent {
                                context::independent_reason(row.key(), catalog).is_none()
                            } else {
                                context::talk_reason(row, catalog, world).is_none()
                            }
                        }
                    }
            })
            .map(LibraryTalk::key)
            .collect::<Vec<_>>(),
    };
    // Available content comes first without changing authored script order.
    keys.sort_by_key(|key| {
        if state.mode == ExperienceMode::Independent {
            return context::independent_reason(*key, catalog).is_some();
        }
        match key {
            EntryKey::Activity(id) => catalog
                .activity(*id)
                .is_none_or(|row| activities::current_reason(row, catalog, world).is_some()),
            EntryKey::Fixture(id) => !world
                .instances
                .get(id)
                .is_some_and(|items| !items.is_empty()),
            _ => catalog
                .talk(*key)
                .is_none_or(|row| !context::talk_here(row, world)),
        }
    });
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transcript_keeps_all_lines_and_speakers() {
        let lines = general_lines(
            &[
                TalkStep::Label {
                    name: "一歌".into(),
                },
                TalkStep::Text {
                    text: "第一句".into(),
                },
                TalkStep::Label { name: "奏".into() },
                TalkStep::Text {
                    text: "很长的第二句\n继续说".into(),
                },
            ],
            "旁白",
            "备用",
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].speaker, "奏");
        assert!(lines[1].text.contains('\n'));
    }
    #[test]
    fn later_lines_are_searchable() {
        let row = make_talk(
            TalkContent {
                master_id: 12,
                backend: TalkBackend::General,
                is_general: Some(true),
            },
            vec![1],
            vec![],
            vec![
                DialogueLine {
                    speaker: "一歌".into(),
                    text: "开场".into(),
                },
                DialogueLine {
                    speaker: "一歌".into(),
                    text: "独特的关键词".into(),
                },
            ],
            false,
            false,
            &LibraryCatalog::default(),
        );
        assert!(query_matches(&row.search, "独特的关键词", 12));
        assert!(!query_matches(&row.search, "123", 12));
        assert!(query_matches(&row.search, "#12", 12));
    }
    #[test]
    fn actor_operand_is_not_a_furniture_master() {
        assert_eq!(
            fixture_step_id(&FixtureStep::LookAtFixture {
                who: 0.5,
                fixture: 1.0
            }),
            None
        );
    }
    #[test]
    fn thumbnail_paths_stay_under_the_manifest_root() {
        assert!(safe_image_path("images/157_1.png"));
        for path in [
            "../private.png",
            "https://example.test/image.png",
            "C:\\secret.png",
            "/absolute.png",
            "images/../a.png",
        ] {
            assert!(!safe_image_path(path));
        }
    }
    #[test]
    fn absent_names_are_not_guessed_from_package_tokens() {
        let mut catalog = LibraryCatalog::default();
        parse_fixtures(
            &serde_json::json!({"fixtures":[{"id":157,"assetbundleName":"mdl_ext0002_fixture_fridge1"}]}),
            &mut catalog,
        );
        assert_eq!(catalog.fixtures[0].name, "未命名家具 · 157");
    }

    #[test]
    fn source_paths_match_by_master_leaf_without_requiring_identical_full_paths() {
        assert_eq!(
            source_leaf("assets/source/mdl_demo_fixture_chair1.prefab"),
            Some("mdl_demo_fixture_chair1")
        );
        assert_eq!(
            source_leaf("mdl_demo_fixture_chair1"),
            Some("mdl_demo_fixture_chair1")
        );
    }

    #[test]
    fn model_index_requires_the_exact_exported_fixture_view_package() {
        let mut catalog = LibraryCatalog::default();
        parse_fixtures(
            &serde_json::json!({"fixtures":[{"id":8,"name":"椅子",
            "assetbundleName":"source/path/mdl_fixture_chair.prefab","gridWidth":2,"gridHeight":2,"gridDepth":2}]}),
            &mut catalog,
        );
        parse_model_index(
            &serde_json::json!({"packages":{"mysekai__fixture__mdl_fixture_chair":{
            "status":"exported","hasFixtureView":true,"glb":"mysekai__fixture__mdl_fixture_chair.glb"}}}),
            &mut catalog,
        );
        assert!(catalog.fixtures[0].source.as_ref().unwrap().exported);
        catalog.fixtures[0].source.as_mut().unwrap().exported = false;
        parse_model_index(
            &serde_json::json!({"packages":{"mysekai__fixture__mdl_fixture_chair":{
            "status":"exported","hasFixtureView":true,"glb":"different.glb"}}}),
            &mut catalog,
        );
        assert!(!catalog.fixtures[0].source.as_ref().unwrap().exported);
    }
}
