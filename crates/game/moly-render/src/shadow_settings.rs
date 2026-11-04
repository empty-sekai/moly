//! Main-light shadow settings: the runtime knobs of the shadow map, read from
//! `moly.toml` at start and changeable while the game runs.
//!
//! [`ShadowSettings`] is a main-world [`Resource`]. It is extracted into the
//! render world ([`ShadowSettingsGlobals`]) on every frame it changed, and that
//! frame is the one that applies the change: the depth maps are re-created
//! (resolution), the light is re-framed (fit, distance), and the consumer
//! block every site and room shell material reads is rewritten (filter,
//! strength, radii, light angle). One extraction covers everything the
//! settings touch, so there is no separate apply step.
//!
//! The channel to the control surface: `moly.graphics.get` answers
//! [`to_json`] and `moly.graphics.set` applies one through [`apply`], which
//! validates all of it or nothing.
//!
//! # Validation
//!
//! [`ShadowSettings::validate`] is the one place that decides whether a set of
//! values can be used. It returns a named refusal (the field, the rule and the
//! value that broke it) for the first field that breaks a rule, and nothing
//! for a set that holds. Bad input is never corrected silently: the start
//! stops with a panic naming the key and the field of a `moly.toml` value, and
//! `moly.graphics.set` answers the refusal and changes nothing.
//!
//! # The consumer block
//!
//! The settings reach the receivers through the consumer block that
//! `shadowmap.rs` writes (`ShadowConsumer`): three vec4s after the matrix,
//! size and params carry the filter, the kernel radii and the depth range.

use bevy::prelude::*;
use bevy::render::{Extract, ExtractSchedule, RenderApp};
use serde_json::Value;

/// How the light's orthographic frame is placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowFit {
    /// A sphere around the camera's frustum slice from its near plane to the
    /// settings' distance: the map follows the camera, so it is dense where
    /// the camera looks. The depth range still spans every caster, so a caster
    /// outside the view casts into the map; a caster outside the sphere's
    /// width does not.
    Camera,
    /// The box around every caster of the site: steady when the camera moves,
    /// but one texel covers the whole site.
    Site,
}

/// How the consumer turns depth comparisons into attenuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowFilter {
    /// One bilinear comparison (the texel footprint the fragment covers).
    Hard,
    /// The nine-tap tent the site program has always used.
    Tent,
    /// A sixteen-tap Vogel disk of world radius `pcf_radius`, turned per pixel
    /// by interleaved gradient noise, each tap a bilinear comparison.
    Pcf,
    /// Contact hardening: a blocker search over the largest penumbra (the map
    /// is read texel by texel for the blockers' depths), then the disk of the
    /// penumbra that the blockers' distance gives.
    Pcss,
}

/// The main-light shadow settings (module comment).
#[derive(Debug, Clone, Copy, PartialEq, Resource)]
pub struct ShadowSettings {
    /// `false`: no depth pass, and every receiver is fully lit.
    pub enabled: bool,
    /// Side of the depth maps, in texels: 512 to 8192, a multiple of 512.
    pub resolution: u32,
    pub fit: ShadowFit,
    /// Camera fit: how far in front of the camera the map reaches, in metres;
    /// the distance fade ends here and starts at 90% of it.
    pub distance: f32,
    pub filter: ShadowFilter,
    /// Pcf: disk radius, in metres.
    pub pcf_radius: f32,
    /// Pcss: angular diameter of the sun, in degrees.
    pub light_angle: f32,
    /// Clamp of every kernel radius, in texels.
    pub max_radius_texels: f32,
    /// The final attenuation is `mix(1, shadow, strength)`.
    pub strength: f32,
    /// Multiplier on the caster's depth bias.
    pub depth_bias: f32,
    /// Multiplier on the caster's normal bias.
    pub normal_bias: f32,
}

/// The allowed resolution range and its grid.
pub const RESOLUTION_MIN: u32 = 512;
pub const RESOLUTION_MAX: u32 = 8192;
pub const RESOLUTION_STEP: u32 = 512;

/// The default resolution: 2560 on a native build, 2048 in the browser.
#[cfg(not(target_arch = "wasm32"))]
pub const DEFAULT_RESOLUTION: u32 = 2560;
#[cfg(target_arch = "wasm32")]
pub const DEFAULT_RESOLUTION: u32 = 2048;

impl Default for ShadowSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            resolution: DEFAULT_RESOLUTION,
            fit: ShadowFit::Camera,
            distance: 30.0,
            filter: ShadowFilter::Pcss,
            pcf_radius: 0.02,
            light_angle: 2.5,
            max_radius_texels: 24.0,
            strength: 1.0,
            depth_bias: 1.0,
            normal_bias: 1.0,
        }
    }
}

/// The `[graphics]` keys, by the name the settings take for them.
const CONFIG_KEYS: [(&str, &moly_config::Key); 11] = [
    ("enabled", &moly_config::keys::SHADOW_ENABLED),
    ("resolution", &moly_config::keys::SHADOW_RESOLUTION),
    ("fit", &moly_config::keys::SHADOW_FIT),
    ("distance", &moly_config::keys::SHADOW_DISTANCE),
    ("filter", &moly_config::keys::SHADOW_FILTER),
    ("pcf_radius", &moly_config::keys::SHADOW_PCF_RADIUS),
    ("light_angle", &moly_config::keys::SHADOW_LIGHT_ANGLE),
    ("max_radius_texels", &moly_config::keys::SHADOW_MAX_RADIUS_TEXELS),
    ("strength", &moly_config::keys::SHADOW_STRENGTH),
    ("depth_bias", &moly_config::keys::SHADOW_DEPTH_BIAS),
    ("normal_bias", &moly_config::keys::SHADOW_NORMAL_BIAS),
];

impl ShadowSettings {
    /// Reads the settings from `moly.toml`: on a native build the `[graphics]`
    /// keys that are set, refused by the key's name when one does not
    /// validate (an invalid key is reported by the configuration's own kind
    /// check first). The browser build has no configuration file and keeps
    /// the defaults.
    pub fn from_config() -> Result<Self, String> {
        let mut settings = Self::default();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let config = moly_config::global();
            let mut values = Vec::new();
            for (name, key) in CONFIG_KEYS {
                let value = match key.kind {
                    moly_config::Kind::Flag => config
                        .flag(key)
                        .map_err(|error| error.to_string())?
                        .map(|flag| if flag { "true" } else { "false" }.to_owned()),
                    moly_config::Kind::Integer => config
                        .integer(key)
                        .map_err(|error| error.to_string())?
                        .map(|number| number.to_string()),
                    moly_config::Kind::Text => config.text(key),
                    // No key of the settings is of these kinds.
                    moly_config::Kind::Path | moly_config::Kind::Url => None,
                };
                if let Some(value) = value {
                    values.push((name.to_owned(), value));
                }
            }
            apply(&mut settings, &values).map_err(|refusal| format!("[graphics] {refusal}"))?;
        }
        Ok(settings)
    }

    /// The first field that breaks a rule, named with the rule and the value;
    /// `None` when every field holds.
    pub fn validate(&self) -> Option<String> {
        if !(RESOLUTION_MIN..=RESOLUTION_MAX).contains(&self.resolution)
            || !self.resolution.is_multiple_of(RESOLUTION_STEP)
        {
            return Some(format!(
                "resolution: must be a multiple of {RESOLUTION_STEP} from {RESOLUTION_MIN} to {RESOLUTION_MAX}, not {}",
                self.resolution
            ));
        }
        let refused = |name: &str, value: f32, holds: bool, rule: &str| -> Option<String> {
            if !value.is_finite() {
                Some(format!("{name}: must be a finite number, not {value}"))
            } else if holds {
                None
            } else {
                Some(format!("{name}: must be {rule}, not {value}"))
            }
        };
        if let Some(found) = refused("distance", self.distance, self.distance > 0.0, "metres above 0") {
            return Some(found);
        }
        if let Some(found) = refused("pcf_radius", self.pcf_radius, self.pcf_radius >= 0.0, "0 or more metres") {
            return Some(found);
        }
        if let Some(found) = refused(
            "light_angle",
            self.light_angle,
            (0.0..90.0).contains(&self.light_angle),
            "from 0 up to 90 degrees",
        ) {
            return Some(found);
        }
        if let Some(found) = refused("max_radius_texels", self.max_radius_texels, self.max_radius_texels >= 1.0, "1 or more texels") {
            return Some(found);
        }
        if !(0.0..=1.0).contains(&self.strength) {
            return Some(format!("strength: must be from 0 to 1, not {}", self.strength));
        }
        if let Some(found) = refused("depth_bias", self.depth_bias, self.depth_bias >= 0.0, "a multiplier of 0 or more") {
            return Some(found);
        }
        if let Some(found) = refused("normal_bias", self.normal_bias, self.normal_bias >= 0.0, "a multiplier of 0 or more") {
            return Some(found);
        }
        None
    }

    /// Sets one field from text (the value as `moly.toml` or JSON gives it);
    /// an unknown name or a value of the wrong type is refused by name. The
    /// rules of the values are [`Self::validate`]'s.
    pub fn set_text(&mut self, name: &str, value: &str) -> Result<(), String> {
        let value = value.trim();
        let flag = || match value {
            "true" | "1" => Ok(true),
            "false" | "0" => Ok(false),
            _ => Err(format!("{name}: must be true or false, not {value:?}")),
        };
        let number = || {
            value
                .parse::<f32>()
                .ok()
                .filter(|number| number.is_finite())
                .ok_or_else(|| format!("{name}: must be a finite number, not {value:?}"))
        };
        match name {
            "enabled" => self.enabled = flag()?,
            "resolution" => {
                self.resolution = value
                    .parse::<u32>()
                    .map_err(|_| format!("{name}: must be a whole number, not {value:?}"))?
            }
            "fit" => {
                self.fit = match value {
                    "camera" => ShadowFit::Camera,
                    "site" => ShadowFit::Site,
                    _ => return Err(format!("{name}: must be camera or site, not {value:?}")),
                }
            }
            "distance" => self.distance = number()?,
            "filter" => {
                self.filter = match value {
                    "hard" => ShadowFilter::Hard,
                    "tent" => ShadowFilter::Tent,
                    "pcf" => ShadowFilter::Pcf,
                    "pcss" => ShadowFilter::Pcss,
                    _ => return Err(format!("{name}: must be hard, tent, pcf or pcss, not {value:?}")),
                }
            }
            "pcf_radius" => self.pcf_radius = number()?,
            "light_angle" => self.light_angle = number()?,
            "max_radius_texels" => self.max_radius_texels = number()?,
            "strength" => self.strength = number()?,
            "depth_bias" => self.depth_bias = number()?,
            "normal_bias" => self.normal_bias = number()?,
            other => return Err(format!("{other} is not a main-light shadow setting")),
        }
        Ok(())
    }

    /// The light size the Pcss filter works from: `tan(angle)` per metre of
    /// blocker-to-receiver distance.
    pub fn penumbra_per_metre(&self) -> f32 {
        self.light_angle.to_radians().tan()
    }
}

/// Applies named values to `settings`: every one is set, then the whole set is
/// validated. All of it holds or none of it is applied, and the refusal names
/// the field. This is what `moly.graphics.set` and the start both use.
pub fn apply(settings: &mut ShadowSettings, values: &[(String, String)]) -> Result<(), String> {
    let before = *settings;
    for (name, value) in values {
        if let Err(refusal) = settings.set_text(name, value) {
            *settings = before;
            return Err(refusal);
        }
    }
    match settings.validate() {
        None => Ok(()),
        Some(refusal) => {
            *settings = before;
            Err(refusal)
        }
    }
}

/// The settings as a JSON object: the shape `moly.graphics.get` answers and
/// `moly.graphics.set` takes under `shadow`.
pub fn to_json(settings: &ShadowSettings) -> Value {
    serde_json::json!({
        "enabled": settings.enabled,
        "resolution": settings.resolution,
        "fit": match settings.fit {
            ShadowFit::Camera => "camera",
            ShadowFit::Site => "site",
        },
        "distance": settings.distance,
        "filter": match settings.filter {
            ShadowFilter::Hard => "hard",
            ShadowFilter::Tent => "tent",
            ShadowFilter::Pcf => "pcf",
            ShadowFilter::Pcss => "pcss",
        },
        "pcf_radius": settings.pcf_radius,
        "light_angle": settings.light_angle,
        "max_radius_texels": settings.max_radius_texels,
        "strength": settings.strength,
        "depth_bias": settings.depth_bias,
        "normal_bias": settings.normal_bias,
    })
}

/// The render world's copy of the settings, written by
/// [`extract_shadow_settings`] when the main-world set changed.
#[derive(Resource, Debug, Clone, Default)]
pub struct ShadowSettingsGlobals {
    pub settings: ShadowSettings,
}

/// The main-light shadow settings: the `[graphics]` values at start, and their
/// copy in the render world. The render world starts with the same values,
/// so the shadow maps are created at the configured resolution.
pub struct ShadowSettingsPlugin;

impl Plugin for ShadowSettingsPlugin {
    fn build(&self, app: &mut App) {
        let settings = ShadowSettings::from_config().unwrap_or_else(|refusal| panic!("moly.toml: {refusal}"));
        app.insert_resource(settings);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .insert_resource(ShadowSettingsGlobals { settings })
            .add_systems(ExtractSchedule, extract_shadow_settings);
    }
}

/// `ExtractSchedule`: copies the main-world settings into the render world on
/// the frames they changed. The copy is written only when the values differ,
/// so the render world sees a change only when there is one.
pub fn extract_shadow_settings(settings: Extract<Res<ShadowSettings>>, mut globals: ResMut<ShadowSettingsGlobals>) {
    if globals.settings != **settings {
        globals.settings = **settings;
    }
}
