//! A compact, source-only payload inventory lets the catalogue reject known
//! unsupported performances before moving scenes. Full dynamic admission is
//! still authoritative; the inventory never declares every asset playable.
use super::*;
use serde_json::Value;
#[derive(Resource)]
pub(crate) struct CapabilityRequest(Handle<JsonAsset>);
#[derive(Resource, Default)]
pub(crate) struct ActivityCapabilities {
    ready: bool,
    prefabs: HashMap<String, Vec<String>>,
}
pub(crate) fn install(app: &mut App) {
    app.init_resource::<ActivityCapabilities>();
}
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(CapabilityRequest(
        server.load("moly://fixture-timeline/capabilities.json"),
    ));
}
pub(crate) fn parse(
    mut commands: Commands,
    request: Option<Res<CapabilityRequest>>,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    mut capabilities: ResMut<ActivityCapabilities>,
    mut catalog: ResMut<LibraryCatalog>,
) {
    let Some(request) = request else {
        return;
    };
    if let LoadState::Failed(error) = server.load_state(&request.0) {
        warn!("[content-library] optional capability inventory unavailable; dynamic admission remains authoritative: {error}");
        capabilities.ready = true;
        commands.remove_resource::<CapabilityRequest>();
        return;
    }
    let Some(asset) = json.get(&request.0) else {
        return;
    };
    match serde_json::from_str::<Value>(&asset.0).ok().and_then(|document|parse_inventory(&document).ok()) {
        Some(rows)=>capabilities.prefabs=rows,
        None=>warn!("[content-library] invalid optional capability inventory; dynamic admission remains authoritative"),
    }
    capabilities.ready = true;
    catalog.revision = catalog.revision.wrapping_add(1);
    commands.remove_resource::<CapabilityRequest>();
}
fn parse_inventory(document: &Value) -> Result<HashMap<String, Vec<String>>, ()> {
    if document["version"].as_u64() != Some(1) {
        return Err(());
    }
    let rows = document["prefabs"].as_array().ok_or(())?;
    let mut result = HashMap::new();
    for row in rows {
        let name = row["prefabName"]
            .as_str()
            .or_else(|| {
                row["prefab"]
                    .as_str()
                    .and_then(|path| path.rsplit('/').next())
                    .map(|name| name.strip_suffix(".prefab").unwrap_or(name))
            })
            .ok_or(())?;
        let mut classes = row["clipClasses"]
            .as_array()
            .ok_or(())?
            .iter()
            .map(|class| class.as_str().map(ToOwned::to_owned).ok_or(()))
            .collect::<Result<Vec<_>, _>>()?;
        if row
            .get("missing")
            .and_then(Value::as_array)
            .is_some_and(|missing| !missing.is_empty())
        {
            classes.push("UnresolvedSourcePayload".into());
        }
        // Duplicate prefab names never erase an earlier unsupported record.
        result
            .entry(name.to_owned())
            .or_insert_with(Vec::new)
            .extend(classes);
    }
    Ok(result)
}
impl ActivityCapabilities {
    pub(super) fn ready(&self) -> bool {
        self.ready
    }
    pub(super) fn reason(&self, logical: &str) -> Option<String> {
        // Target-height suffix insertion comes from timeline_asset. Looking at
        // every actual exported variant prevents banning a supported ground
        // performance because an unrelated high-table variant has extra effects.
        let variants: Vec<_> = self
            .prefabs
            .iter()
            .filter(|(name, _)| name.replace("-ground", "").replace("-low", "") == logical)
            .collect();
        if variants.is_empty() {
            return None;
        }
        if variants
            .iter()
            .all(|(_, classes)| classes.iter().any(|class| !supported(class)))
        {
            Some("这段演出包含暂未支持的家具效果，可以先欣赏其他互动。".into())
        } else {
            None
        }
    }
}
fn supported(class: &str) -> bool {
    matches!(
        class,
        "AnimationPlayableAsset"
            | "ControlPlayableAsset"
            | "ChangeEyePresetClip"
            | "ChangeLipSyncPresetClip"
            | "LoopFlagClip"
            | "SEClip"
            | "ChangeBlinkStateClip"
            | "ChangeLipSyncStateClip"
            | "EnableIKTalkClip"
            | "EmoticonClip"
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_known_unsupported_complete_variant_sets_are_disabled() {
        let mut inventory = ActivityCapabilities {
            ready: true,
            ..default()
        };
        inventory.prefabs.insert(
            "tl_chair-ground_w".into(),
            vec!["UnresolvedSourcePayload".into()],
        );
        assert!(inventory.reason("tl_chair_w").is_some());
        inventory.prefabs.insert(
            "tl_chair-low_w".into(),
            vec!["AnimationPlayableAsset".into(), "EmoticonClip".into()],
        );
        assert!(inventory.reason("tl_chair_w").is_none());
        assert!(inventory.reason("tl_unknown_w").is_none());
    }

    #[test]
    fn source_control_particles_reach_dynamic_binding_validation() {
        let inventory = ActivityCapabilities {
            ready: true,
            prefabs: HashMap::from([
                ("tl_effect-ground_m".into(), vec!["AnimationPlayableAsset".into(), "ControlPlayableAsset".into()]),
                ("tl_other-ground_m".into(), vec!["ControlPlayableAsset".into(), "FadeCharacterClip".into()]),
            ]),
        };
        assert!(inventory.reason("tl_effect_m").is_none());
        assert!(inventory.reason("tl_other_m").is_some());
    }
}
