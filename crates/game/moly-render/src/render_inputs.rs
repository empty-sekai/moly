//! State that the world and the game publish for the renderer: what the render
//! systems read about sites, furniture and characters. The publishing systems
//! live with the domains that own the state.

use bevy::gltf::Gltf;
use bevy::prelude::*;

/// Which site the environment view is showing: the source's indoor predicate
/// and the scene name the site's colliders are reported under. The site loader
/// fills it from its own record every frame it has one, so a render system
/// reads the same value at the same point of the frame.
#[derive(Clone, Resource)]
pub struct SiteView {
    /// The active site's id.
    pub site_id: u32,
    /// The scene directory name of the active site.
    pub scene: String,
    /// The active site type and category (`housing_home`, `harvest`, ...).
    pub site_type: String,
    pub category: String,
    /// The site is one of the indoor floors: the weather effects are hidden
    /// and the plain background is shown on it.
    pub indoor: bool,
    /// The site's source-world offset.
    pub position: [f32; 3],
    /// The environment site name: the phenomenon packages pick their
    /// `unique__<site>` variant by it.
    pub env_site: String,
}

impl Default for SiteView {
    fn default() -> Self {
        Self {
            site_id: 0,
            scene: String::new(),
            site_type: String::new(),
            category: String::new(),
            indoor: false,
            position: [0.0; 3],
            env_site: String::new(),
        }
    }
}

/// 家具根实体标记（一条摆放一个）。The renderer reads furniture roots; the
/// world's loader owns their lifetime.
#[derive(Component)]
pub struct FixtureRoot;

/// 家具根上的源 glb 句柄（装载期留底）：对话演出与粒子控制要按它取该
/// 包的动画剪辑集与粒子档案。
#[derive(Component)]
pub struct FixtureSource(pub Handle<Gltf>);

/// 全部摆放的 scene 展开完毕（换装的门）；由站点家具域计数置位。
#[derive(Resource)]
pub struct FixtureScenesReady;

/// Rendering ownership is independent of committed gameplay placement.
/// Editor previews opt into the same material chain without a fixture root,
/// placement, occupancy or interaction identity.
#[derive(Component)]
pub struct FixtureVisualRoot {
    pub layout: u8,
}

/// A fixture view whose scene has expanded; the colour and material passes
/// wait for it.
#[derive(Component)]
pub struct FixtureVisualSceneReady;

/// The placed fixtures' colliders, in source axes, as the particle physics
/// scene carries them: which site and layout generation they belong to, and
/// either the built scene or the reason it is not built yet.
#[derive(Resource)]
pub struct FixtureCollisionScene {
    /// The site the rows belong to.
    pub site_id: u32,
    /// The layout generation the rows were placed against.
    pub revision: u64,
    /// The number of placed rows.
    pub rows: usize,
    /// `None` until the colliders are built; the builder retries every frame
    /// while it stays `None`.
    pub built: Option<std::sync::Arc<crate::particle_runtime::collision_scene::GroundScene>>,
    /// Why the colliders are not built, for the host's status line.
    pub error: Option<String>,
}

impl FixtureVisualRoot {
    /// 墙布局判定：LayoutType 命中 0xF0 任一位。
    pub fn is_wall_layout(&self) -> bool {
        self.layout & moly_law::fixture::position::WALL_LAYOUT_MASK != 0
    }
}

/// All local one-shot binders reset against this generation when a map's
/// furniture instances are replaced. Source catalogs remain reusable.
#[derive(Resource, Default)]
pub struct FixtureLayoutRevision(pub u64);

/// The system set of the site furniture loader's per-frame chain: the material
/// and colour passes read a layout after the loader has placed it.
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct FixtureLayoutSet;

/// A fixture view whose source materials are installed and whose root is shown.
#[derive(Component)]
pub struct FixtureVisualReady;

/// The cannon move's writes to the source's environment root, for the sky
/// host that keeps that root. At the fire the move keeps the root's height
/// (`envDefaultPositionY`); on every update of the follow (`DOVirtual.Float`
/// over the flight time) it sets the root to the player view's `Step`
/// position; at the landing effect it sets the root's height back to the
/// kept one. Points are in the source world frame. Present from the fire to
/// the end of the move; door moves do not write the root. The product tweens
/// the player's root, which the `Step` node sits on.
#[derive(Resource, Clone, Debug)]
pub struct CannonEnvironmentWrite {
    /// One value per move (its admission frame).
    pub move_id: u64,
    /// The destination's site type.
    pub site_type: String,
    /// The follow's last write; `None` before its first update.
    pub point: Option<Vec3>,
    /// The landing effect has set the kept height back.
    pub height_restored: bool,
}

/// Whether the committed post profile's bloom is on. The weather domain owns
/// the profile; the emission node reads only this gate before it decides
/// whether its target is observable.
#[derive(Clone, Copy, Resource, bevy::render::extract_resource::ExtractResource, Default)]
#[extract_app(bevy::render::RenderApp)]
pub struct PostBloomInput {
    pub bloom_on: bool,
}

/// The settings panel's request to release unused render-cache handles, and
/// the renderer's answer for the panel's status line. The panel raises the
/// request; the render modules that own the caches clear their own and write
/// their count here for the panel to show.
#[derive(Resource, Default)]
pub struct RenderCacheClear {
    pub requested: bool,
    /// One line per cache that answered, in the order the answers arrived.
    pub status: Option<String>,
}

impl RenderCacheClear {
    /// Records an answer: the number of materials and textures released.
    pub fn released(&mut self, materials: usize, images: usize) {
        self.status = Some(format!(
            "已释放未使用缓存句柄：材质 {materials}，纹理 {images}（GPU 回收由引擎异步完成）"
        ));
    }
}

/// A render module's request for the settings panel to show the player-data
/// page with this error. The panel reads it in the same frame, shows itself
/// and writes the text to the import's status line, then clears it.
#[derive(Resource, Default)]
pub struct OpenPlayerDataPanel {
    pub status: Option<String>,
}

/// `ControlPlayableAsset`'s own fields, as the particle binder resolves them
/// against the bound source object: the exposed name, which child kinds the
/// clip updates, whether the hierarchy is searched and the playback flags.
#[derive(Clone, Debug)]
pub struct ControlSettings {
    pub exposed_name: String,
    pub update_particle: bool,
    pub update_director: bool,
    pub update_itime_control: bool,
    pub search_hierarchy: bool,
    pub active: bool,
    pub post_playback: u64,
    pub random_seed: u32,
}

/// Why a timeline row could not be admitted. `retryable` is true only for a
/// requested asset that is still in flight; a structural, source-identity or
/// failed-load error must not consume a timeout budget.
#[derive(Clone, Debug)]
pub struct TimelineFailure {
    pub message: String,
    pub retryable: bool,
}

impl TimelineFailure {
    pub fn loading(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }
}
impl std::fmt::Display for TimelineFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for TimelineFailure {}

/// The active site's main glTF handle: the material swap joins the sidecar's
/// material table against the glb's own table by index, so it reads the handle
/// the loader asked for. The site loader publishes it with the record.
#[derive(Clone, Resource)]
pub struct SiteSceneGltf {
    pub gltf: Handle<Gltf>,
}

/// `EffectManager`'s `EffectType` values the product plays by name. The site
/// move's three come from the harvest action family, whose packages ship a
/// prefab glb; the player's two foot effects come from the common action
/// family, whose packages hold no mesh and so ship no glb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectType {
    SiteMoveEndPlayer = 4,
    SiteMoveFailedPlayer = 5,
    Dash = 14,
    Flying = 16,
    WalkWater = 17,
}

impl EffectType {
    /// The package's last name segment (`EffectResourceData` bundle names).
    fn leaf(self) -> &'static str {
        match self {
            Self::SiteMoveEndPlayer => "fx_act_user_landing",
            Self::SiteMoveFailedPlayer => "fx_act_user_landing_fail",
            Self::Flying => "fx_act_user_flying",
            Self::Dash => "dash",
            Self::WalkWater => "walk_water",
        }
    }

    /// The prefab `EffectResourceData` names in the package; its root node.
    pub fn prefab(self) -> &'static str {
        match self {
            Self::Dash => "fx_act_user_walking",
            Self::WalkWater => "fx_act_user_walking_water",
            other => other.leaf(),
        }
    }

    pub fn package(self) -> String {
        match self {
            Self::Dash | Self::WalkWater => {
                format!("mysekai__effect__site__common__action__{}", self.leaf())
            }
            _ => format!("mysekai__effect__site__harvest__action__{}", self.leaf()),
        }
    }

    pub fn doc_path(self) -> String {
        format!("moly://fixture-particles-v2/{}.json", self.package())
    }
}

/// A real hidden resident view still needs material preparation. This marker
/// prevents that preparation from making its render root visible.
#[derive(Component)]
pub struct HiddenResidentVisual;
