//! Site expansion dissolve: the global keyword `_USE_MYSEKAI_SITE_EXTENSION`
//! and the nine `_SiteExtension*` globals.
//!
//! Source structure, reproduced here:
//!
//! - `SiteEnvironmentViewManager.SetSiteExtension(data)` only stores the
//!   struct in the environment view controller; it writes no shader state.
//!   `ResetGlobalDissolve()` stores `EnvironmentSiteExtensionData.Default`
//!   the same way. [`SiteExtension::set_site_extension`] and
//!   [`SiteExtension::reset_global_dissolve`] are those two calls.
//! - Every frame the view controller copies the stored struct into the
//!   environment shader view model, and the environment shader view then
//!   enables the keyword exactly when `IsActive` is set and writes all nine
//!   globals whether the keyword is on or off. [`copy_to_shader_view`] is
//!   that per-frame copy; the render world writes the copy into one uniform
//!   buffer that the site and room shell materials bind.
//!
//! The copy runs in `First`, before the frame's `Update` systems (the source
//! copies from the view manager's `Update`), so a value stored during the
//! `Update` of frame N is drawn from frame N + 1.
//!
//! Readers of the globals: the programs of `Mysekai/Object` (every usage;
//! the edge arm only for usage 8), `Mysekai/Site/Ground`, `Mysekai/Water`,
//! `Mysekai/Site/Ground-Birthday` (their phenomena-lit variants) and
//! `Mysekai/TreasureBox`. The keyword is declared by the Tree, FieldObject
//! and fixture shaders too, but their programs with and without it are the
//! same text, so those materials do not read the buffer.
//! `_SiteExtensionLimitLineWidth` is written every frame and declared by no
//! program; the buffer carries it so the table stays whole.

use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_resource::{Buffer, BufferInitDescriptor, BufferUsages};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

/// `EnvironmentSiteExtensionData`, field for field, in the source's
/// coordinates (Unity world space; the product's world is the same space
/// mirrored in X, see [`SiteExtensionGlobals::gpu_bytes`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SiteExtensionData {
    /// `IsActive`: the keyword is enabled exactly while this is set.
    pub is_active: bool,
    /// `CenterPosition` (world space).
    pub center_position: [f32; 3],
    /// `DissolveSmoothness` (`_SiteExtensionSmoothness`).
    pub dissolve_smoothness: f32,
    /// `DissoveEdgeColor` (`_SiteExtensionEdgeColor`), as the source spells it.
    pub dissolve_edge_color: [f32; 4],
    /// `FadeColor` (`_SiteExtensionFadeColor`).
    pub fade_color: [f32; 4],
    /// `Radius` (`_SiteExtensionRadius`).
    pub radius: f32,
    /// `InnerRadius` (`_SiteExtensionInnerRadius`).
    pub inner_radius: f32,
    /// `FadeMinRadius` (`_SiteExtensionFadeMinRadius`).
    pub fade_min_radius: f32,
    /// `FadeMaxRadius` (`_SiteExtensionFadeMaxRadius`).
    pub fade_max_radius: f32,
    /// `LimitLineWidth` (`_SiteExtensionLimitLineWidth`).
    pub limit_line_width: f32,
}

impl SiteExtensionData {
    /// `EnvironmentSiteExtensionData.Default`, the static initializer's
    /// values: inactive, centre at the origin, smoothness 0.2, edge and fade
    /// colours (0, 1, 1, 1), radii 0, fade range [0, 10], limit width 0.
    pub const DEFAULT: Self = Self {
        is_active: false,
        center_position: [0.0, 0.0, 0.0],
        dissolve_smoothness: 0.2,
        dissolve_edge_color: [0.0, 1.0, 1.0, 1.0],
        fade_color: [0.0, 1.0, 1.0, 1.0],
        radius: 0.0,
        inner_radius: 0.0,
        fade_min_radius: 0.0,
        fade_max_radius: 10.0,
        limit_line_width: 0.0,
    };

    /// The struct `ShowExpansionEffectBehaviour.ProcessFrame` passes to
    /// `SetSiteExtension` for a `ShowExpansionEffectClip` at playable time
    /// `time` of `duration` seconds.
    ///
    /// Progress is `time / duration` in double precision, rounded to single;
    /// a negative progress is 0, anything else is `min(progress, 1)` with a
    /// NaN kept as NaN. The radius is `start + progress * (end - start)` in
    /// that operation order, the inner radius `radius - max(gradient, 0)`
    /// (NaN kept). Smoothness and limit line width are the `Default` values;
    /// the centre and both colours are the clip's.
    #[allow(clippy::too_many_arguments)]
    pub fn expansion_frame(
        center_position: [f32; 3],
        start_radius: f32,
        end_radius: f32,
        min_radius: f32,
        max_radius: f32,
        gradient_range: f32,
        edge_color: [f32; 4],
        fade_color: [f32; 4],
        time: f64,
        duration: f64,
    ) -> Self {
        let p = (time / duration) as f32;
        // `p >= 0` or unordered takes the minimum; the minimum propagates NaN.
        let progress = if p.is_nan() {
            p
        } else if p >= 0.0 {
            p.min(1.0)
        } else {
            0.0
        };
        let radius = start_radius + progress * (end_radius - start_radius);
        // The maximum propagates NaN and orders +0 above -0.
        let gradient = if gradient_range.is_nan() {
            gradient_range
        } else if gradient_range > 0.0 {
            gradient_range
        } else {
            0.0
        };
        Self {
            is_active: true,
            center_position,
            dissolve_smoothness: Self::DEFAULT.dissolve_smoothness,
            dissolve_edge_color: edge_color,
            fade_color,
            radius,
            inner_radius: radius - gradient,
            fade_min_radius: min_radius,
            fade_max_radius: max_radius,
            limit_line_width: Self::DEFAULT.limit_line_width,
        }
    }
}

impl Default for SiteExtensionData {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The view controller's stored struct: what `SetSiteExtension` and
/// `ResetGlobalDissolve` write.
#[derive(Resource, Debug, Clone, Default)]
pub struct SiteExtension {
    stored: SiteExtensionData,
}

impl SiteExtension {
    /// `SiteEnvironmentViewManager.SetSiteExtension`.
    pub fn set_site_extension(&mut self, data: SiteExtensionData) {
        self.stored = data;
    }

    /// `SiteEnvironmentViewManager.ResetGlobalDissolve`.
    pub fn reset_global_dissolve(&mut self) {
        self.stored = SiteExtensionData::DEFAULT;
    }

    /// The stored struct (the next frame's globals).
    pub fn stored(&self) -> SiteExtensionData {
        self.stored
    }
}

/// [`SiteExtension::set_site_extension`] for callers that hold the world.
pub fn set_site_extension(world: &mut World, data: SiteExtensionData) {
    world
        .resource_mut::<SiteExtension>()
        .set_site_extension(data);
}

/// [`SiteExtension::reset_global_dissolve`] for callers that hold the world.
pub fn reset_global_dissolve(world: &mut World) {
    world
        .resource_mut::<SiteExtension>()
        .reset_global_dissolve();
}

/// The environment shader view's copy: the keyword state and the nine
/// globals of this frame.
#[derive(Resource, Debug, Clone, Default, ExtractResource)]
pub struct SiteExtensionGlobals {
    pub data: SiteExtensionData,
}

/// GPU layout: 5 vec4 slots, 80 bytes. The slot order is the contract with
/// the `SiteExtension` struct of `shaders/site_material.wgsl` and
/// `shaders/room_shell.wgsl`.
pub const SITE_EXTENSION_SLOTS: usize = 5;
pub const SITE_EXTENSION_BYTES: usize = SITE_EXTENSION_SLOTS * 16;

impl SiteExtensionGlobals {
    /// Slots:
    /// 0 `(_SiteExtensionRadius, _SiteExtensionInnerRadius,
    ///   _SiteExtensionSmoothness, keyword)`, keyword 1 when enabled;
    /// 1 `_SiteExtensionCenter`: the programs declare a vec3 and read x and
    ///   z; the source writes it with `SetGlobalVector(x, y, z, 0)`. The x
    ///   component is mirrored into the product's world here, the one X
    ///   reflection between the source's world and the product's;
    /// 2 `_SiteExtensionEdgeColor` (rgba);
    /// 3 `_SiteExtensionFadeColor` (rgba);
    /// 4 `(_SiteExtensionFadeMinRadius, _SiteExtensionFadeMaxRadius,
    ///   _SiteExtensionLimitLineWidth, 0)`.
    ///
    /// Colours go in as stored values: the source is a gamma player, where
    /// `SetGlobalColor` converts nothing, and the site programs work in the
    /// stored domain.
    pub fn gpu_bytes(&self) -> Vec<u8> {
        let d = &self.data;
        let mut bytes = Vec::with_capacity(SITE_EXTENSION_BYTES);
        let keyword = if d.is_active { 1.0 } else { 0.0 };
        let c = d.center_position;
        for slot in [
            [d.radius, d.inner_radius, d.dissolve_smoothness, keyword],
            [-c[0], c[1], c[2], 0.0],
            d.dissolve_edge_color,
            d.fade_color,
            [
                d.fade_min_radius,
                d.fade_max_radius,
                d.limit_line_width,
                0.0,
            ],
        ] {
            for component in slot {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        debug_assert_eq!(bytes.len(), SITE_EXTENSION_BYTES);
        bytes
    }
}

/// `First`: the view controller's per-frame copy into the shader view.
pub fn copy_to_shader_view(
    extension: Res<SiteExtension>,
    mut globals: ResMut<SiteExtensionGlobals>,
) {
    let stored = extension.stored;
    if globals.data == stored {
        return;
    }
    if globals.data.is_active != stored.is_active {
        info!(
            "site extension: keyword _USE_MYSEKAI_SITE_EXTENSION {}",
            if stored.is_active {
                "enabled"
            } else {
                "disabled"
            }
        );
    }
    globals.data = stored;
}

/// Render-world buffer of the globals, bound by the site and room shell
/// materials.
#[derive(Resource)]
pub struct SiteExtensionGpuBuffer {
    pub buffer: Buffer,
}

fn create_buffer(mut commands: Commands, render_device: Res<RenderDevice>) {
    let buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("site_extension"),
        contents: &SiteExtensionGlobals::default().gpu_bytes(),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    });
    commands.insert_resource(SiteExtensionGpuBuffer { buffer });
}

fn write_buffer(
    globals: Option<Res<SiteExtensionGlobals>>,
    buffer: Res<SiteExtensionGpuBuffer>,
    queue: Res<RenderQueue>,
) {
    // Extraction replaces the render-world copy only on a frame where the
    // main-world copy changed; the buffer already holds every earlier value.
    let Some(globals) = globals else {
        return;
    };
    if !globals.is_changed() {
        return;
    }
    queue.write_buffer(&buffer.buffer, 0, &globals.gpu_bytes());
}

/// The stored struct, the per-frame copy and the render-world buffer.
pub struct SiteExtensionPlugin;

impl Plugin for SiteExtensionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SiteExtension>()
            .init_resource::<SiteExtensionGlobals>()
            .add_plugins(ExtractResourcePlugin::<SiteExtensionGlobals>::default())
            .add_systems(First, copy_to_shader_view)
            .add_systems(Update, dev::play.run_if(crate::dev_tools::dev_tools));
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .add_systems(RenderStartup, create_buffer)
                .add_systems(Render, write_buffer.in_set(RenderSystems::Prepare));
        }
    }
}

/// Developer hook: plays one expansion clip through the public calls, with
/// every value taken from `MOLY_SITE_EXTENSION_DEV` (no clip data lives in
/// the code). The variable is `;`-separated `key=value` pairs:
/// `start` (seconds of virtual time at which the clip starts), `duration`,
/// `hold` (seconds after the clip ends before `ResetGlobalDissolve`; the
/// last frame's struct stays stored meanwhile), `center=x,y,z`,
/// `radius=start,end`, `fade=min,max`, `gradient`, `edge=r,g,b,a`,
/// `fade_color=r,g,b,a`. A missing or unknown key panics: the hook refuses
/// to guess a value.
mod dev {
    use super::{SiteExtension, SiteExtensionData};
    use bevy::prelude::*;

    pub(super) struct Clip {
        start: f64,
        duration: f64,
        hold: f64,
        center: [f32; 3],
        radius: [f32; 2],
        fade: [f32; 2],
        gradient: f32,
        edge: [f32; 4],
        fade_color: [f32; 4],
    }

    fn floats<const N: usize>(key: &str, value: &str) -> [f32; N] {
        let parts: Vec<f32> = value
            .split(',')
            .map(|part| {
                part.trim().parse::<f32>().unwrap_or_else(|_| {
                    panic!("MOLY_SITE_EXTENSION_DEV {key}={value}: not a number list")
                })
            })
            .collect();
        parts.try_into().unwrap_or_else(|parts: Vec<f32>| {
            panic!(
                "MOLY_SITE_EXTENSION_DEV {key}: {} values, expected {N}",
                parts.len()
            )
        })
    }

    pub(super) fn parse(spec: &str) -> Clip {
        let mut pairs = std::collections::HashMap::new();
        for pair in spec.split(';').filter(|pair| !pair.trim().is_empty()) {
            let (key, value) = pair
                .split_once('=')
                .unwrap_or_else(|| panic!("MOLY_SITE_EXTENSION_DEV: `{pair}` is not key=value"));
            pairs.insert(key.trim().to_string(), value.trim().to_string());
        }
        const KEYS: [&str; 9] = [
            "start",
            "duration",
            "hold",
            "center",
            "radius",
            "fade",
            "gradient",
            "edge",
            "fade_color",
        ];
        if let Some(unknown) = pairs.keys().find(|key| !KEYS.contains(&key.as_str())) {
            panic!("MOLY_SITE_EXTENSION_DEV: unknown key `{unknown}`");
        }
        let get = |key: &str| -> &str {
            pairs
                .get(key)
                .unwrap_or_else(|| panic!("MOLY_SITE_EXTENSION_DEV: missing key `{key}`"))
        };
        let seconds = |key: &str| -> f64 {
            get(key).parse::<f64>().unwrap_or_else(|_| {
                panic!("MOLY_SITE_EXTENSION_DEV {key}={}: not a number", get(key))
            })
        };
        let [gradient] = floats::<1>("gradient", get("gradient"));
        Clip {
            start: seconds("start"),
            duration: seconds("duration"),
            hold: seconds("hold"),
            center: floats("center", get("center")),
            radius: floats("radius", get("radius")),
            fade: floats("fade", get("fade")),
            gradient,
            edge: floats("edge", get("edge")),
            fade_color: floats("fade_color", get("fade_color")),
        }
    }

    #[derive(Default)]
    pub(super) struct State {
        clip: Option<Option<Clip>>,
        frames: u32,
        reset: bool,
    }

    pub(super) fn play(
        time: Res<Time>,
        mut extension: ResMut<SiteExtension>,
        mut state: Local<State>,
    ) {
        let State {
            clip,
            frames,
            reset,
        } = &mut *state;
        let clip = clip.get_or_insert_with(|| {
            std::env::var("MOLY_SITE_EXTENSION_DEV")
                .ok()
                .map(|spec| parse(&spec))
        });
        let Some(clip) = clip else {
            return;
        };
        if *reset {
            return;
        }
        let now = time.elapsed_secs_f64();
        let local = now - clip.start;
        if local < 0.0 {
            return;
        }
        if local <= clip.duration {
            let data = SiteExtensionData::expansion_frame(
                clip.center,
                clip.radius[0],
                clip.radius[1],
                clip.fade[0],
                clip.fade[1],
                clip.gradient,
                clip.edge,
                clip.fade_color,
                local,
                clip.duration,
            );
            extension.set_site_extension(data);
            *frames += 1;
            info!(
                "site extension dev: t={local:.4} SetSiteExtension(radius {:.4}, inner {:.4}, fade [{}, {}], smoothness {})",
                data.radius, data.inner_radius, data.fade_min_radius, data.fade_max_radius,
                data.dissolve_smoothness
            );
        } else if local > clip.duration + clip.hold {
            extension.reset_global_dissolve();
            *reset = true;
            info!(
                "site extension dev: ResetGlobalDissolve after {} SetSiteExtension frames",
                frames
            );
        }
    }
}
