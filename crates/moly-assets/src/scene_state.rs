//! Authored object activity and renderer visibility on actual scene instances.
//!
//! Source activity is independent of render visibility: a navigation surface
//! can be intentionally invisible yet active, while a dormant object must not
//! contribute geometry, sound or particle simulation. Deactivation retains the
//! hierarchy and its bindings so subsequent activation can use the same entity.

use bevy::{asset::LoadContext, gltf::extensions::GltfExtensionHandler, prelude::*};

#[derive(Component, Reflect, Clone, Copy, Debug)]
#[reflect(Component)]
pub struct SourceNodeActivity(bool);

impl SourceNodeActivity {
    pub fn active_self(&self) -> bool {
        self.0
    }
}

/// Derived activity in the source hierarchy, not camera/frustum visibility.
#[derive(Component, Reflect, Clone, Copy, Debug, Default)]
#[reflect(Component)]
pub struct SourceInactive;

/// Unity Rendering.ShadowCastingMode — serialized value is the variant index.
/// Shader ShadowCaster pass availability remains a separate material property.
#[derive(Reflect, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceShadowCastingMode {
    Off,
    On,
    TwoSided,
    ShadowsOnly,
}

impl SourceShadowCastingMode {
    fn from_serialized(value: u64) -> Option<Self> {
        match value {
            0 => Some(Self::Off),
            1 => Some(Self::On),
            2 => Some(Self::TwoSided),
            3 => Some(Self::ShadowsOnly),
            _ => None,
        }
    }
}

#[derive(Component, Reflect, Clone, Copy, Debug)]
#[reflect(Component)]
pub struct SourceRenderer {
    enabled: bool,
    material_assigned: bool,
    // None identifies a legacy export without renderer mode metadata.
    shadow_casting: Option<SourceShadowCastingMode>,
}

impl SourceRenderer {
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn material_assigned(&self) -> bool {
        self.material_assigned
    }
    pub fn shadow_casting(&self) -> Option<SourceShadowCastingMode> {
        self.shadow_casting
    }
    pub fn shadows_only(&self) -> bool {
        self.shadow_casting == Some(SourceShadowCastingMode::ShadowsOnly)
    }
    pub fn casts_shadows(&self) -> bool {
        self.enabled
            && self.material_assigned
            && self.shadow_casting != Some(SourceShadowCastingMode::Off)
    }
}

/// Reconcile a newly instanced or reparented subtree with its actual parent.
pub struct RefreshSourceActivity(pub Entity);

impl Command for RefreshSourceActivity {
    fn apply(self, world: &mut World) {
        if world.get_entity(self.0).is_ok() {
            propagate_activity(world, self.0);
        }
    }
}

/// Change activity and its inherited effects atomically through Commands.
/// Keeping the stored field private prevents changing it without propagation.
pub struct SetSourceActive {
    pub entity: Entity,
    pub active: bool,
}

impl Command for SetSourceActive {
    fn apply(self, world: &mut World) {
        let Some(current) = world.get::<SourceNodeActivity>(self.entity) else {
            return;
        };
        if current.0 == self.active {
            return;
        }
        world.entity_mut(self.entity).insert((
            SourceNodeActivity(self.active),
            if self.active {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            },
        ));
        propagate_activity(world, self.entity);
    }
}

/// Renderer enablement does not deactivate the object's children or logic.
pub struct SetSourceRendererEnabled {
    pub node: Entity,
    pub enabled: bool,
}

impl Command for SetSourceRendererEnabled {
    fn apply(self, world: &mut World) {
        let Some(mut renderer) = world.get::<SourceRenderer>(self.node).copied() else {
            return;
        };
        renderer.enabled = self.enabled;
        world.entity_mut(self.node).insert(renderer);
        apply_renderer(world, self.node, renderer);
    }
}

/// Update both colour eligibility and the custom shadow draw's source state.
pub struct SetSourceShadowCastingMode {
    pub node: Entity,
    pub mode: SourceShadowCastingMode,
}

impl Command for SetSourceShadowCastingMode {
    fn apply(self, world: &mut World) {
        let Some(mut renderer) = world.get::<SourceRenderer>(self.node).copied() else {
            return;
        };
        renderer.shadow_casting = Some(self.mode);
        world.entity_mut(self.node).insert(renderer);
        apply_renderer(world, self.node, renderer);
    }
}

fn propagate_activity(world: &mut World, root: Entity) {
    let mut parent = world.get::<ChildOf>(root).map(ChildOf::parent);
    let mut inherited = true;
    while let Some(entity) = parent {
        inherited &= world
            .get::<SourceNodeActivity>(entity)
            .is_none_or(|state| state.0);
        parent = world.get::<ChildOf>(entity).map(ChildOf::parent);
    }
    let mut stack = vec![(root, inherited)];
    while let Some((entity, inherited)) = stack.pop() {
        let active = inherited
            && world
                .get::<SourceNodeActivity>(entity)
                .is_none_or(|state| state.0);
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter().map(|child| (child, active)));
        }
        if active {
            world.entity_mut(entity).remove::<SourceInactive>();
        } else {
            world.entity_mut(entity).insert(SourceInactive);
        }
    }
}

fn apply_renderer(world: &mut World, node: Entity, renderer: SourceRenderer) {
    // Bevy creates primitives directly below their glTF object node. Only
    // those primitives belong to this Renderer, not other child objects.
    let children: Vec<Entity> = world
        .get::<Children>(node)
        .map(|children| children.iter().collect())
        .unwrap_or_default();
    for entity in children {
        if world.get::<Mesh3d>(entity).is_none() {
            continue;
        }
        world.entity_mut(entity).insert((
            renderer,
            if renderer.enabled && renderer.material_assigned && !renderer.shadows_only() {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            },
        ));
    }
}

#[derive(Default)]
pub(crate) struct SceneStateLoader;

impl GltfExtensionHandler for SceneStateLoader {
    fn dyn_clone(&self) -> Box<dyn GltfExtensionHandler> {
        Box::new(Self)
    }

    fn on_gltf_node(
        &mut self,
        _context: &mut LoadContext<'_>,
        node: &gltf::Node,
        entity: &mut EntityWorldMut,
    ) {
        let extras = node
            .extras()
            .as_ref()
            .and_then(|extras| serde_json::from_str::<serde_json::Value>(extras.get()).ok());
        if let Some(extras) = &extras {
            crate::coordinates::import(extras, entity);
            crate::source_navigation::import(extras, entity);
        }
        // Fence exports predate the generic site node-state spelling. Both
        // encode the same authored object/renderer fields; do not combine them.
        let flag = |names: &[&str]| {
            extras
                .as_ref()
                .and_then(|value| names.iter().find_map(|name| value.get(*name)))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true)
        };
        let active = flag(&["active", "fenceActive"]);
        entity.insert(SourceNodeActivity(active));
        if !active {
            entity.insert(Visibility::Hidden);
        }
        if node.mesh().is_some() {
            let renderer = SourceRenderer {
                enabled: flag(&["enabled", "fenceRendererEnabled"]),
                material_assigned: flag(&["materialed"]),
                shadow_casting: extras
                    .as_ref()
                    .and_then(|value| value.get("shadowCastingMode"))
                    .map(|value| {
                        value
                            .as_u64()
                            .and_then(SourceShadowCastingMode::from_serialized)
                            .expect("invalid exported Renderer.shadowCastingMode")
                    }),
            };
            entity.insert(renderer);
            let id = entity.id();
            entity.world_scope(|world| apply_renderer(world, id, renderer));
        }
    }

    fn on_scene_completed(
        &mut self,
        _context: &mut LoadContext<'_>,
        _scene: &gltf::Scene,
        world_root_id: Entity,
        scene_world: &mut World,
    ) {
        propagate_activity(scene_world, world_root_id);
    }
}

pub(crate) fn register_types(app: &mut App) {
    app.register_type::<crate::coordinates::CanonicalCoordinates>();
    crate::source_navigation::register(app);
    app.register_type::<SourceNodeActivity>()
        .register_type::<SourceInactive>()
        .register_type::<SourceRenderer>()
        .register_type::<SourceShadowCastingMode>()
        .add_observer(
            |event: On<bevy::scene::SceneInstanceReady>, mut commands: Commands| {
                // The asset's isolated Scene world cannot know whether its eventual
                // instance is attached beneath an inactive object in the game.
                commands.queue(RefreshSourceActivity(event.event().entity));
            },
        );
}
