//! The environment loader, visual fade and global FX commit have separate identities.
//! A cancelled visual fade reaches its end value without committing its playable,
//! post profile or global effects. A replacement waits for its own loaded assets.
//!
//! The fade duration belongs to the caller of `SiteEnvironmentManager.CrossFade`: a
//! phenomenon change at the same site passes the home literal, a site move passes
//! what its move state passes (see [`EnvironmentMove`]). `DoCrossFadeAsync` runs
//! `while (time < fadeTime)`, so a zero duration skips the loop and writes the end
//! progress at once; the profile, shader data and global effects then commit in the
//! same continuation.
use bevy::prelude::*;
use serde::Serialize;

/// HomeSiteController and SiteLayoutEditor explicitly pass this duration.
/// EnvironmentCrossFieldParam's constructor default (1 second) is not this callsite.
pub(crate) const HOME_ENVIRONMENT_FADE_SECONDS: f32 = 0.25;

/// `MoveSiteUseCannonActionState.ChangeEnvironment` passes
/// `MysekaiGraphicsConfig.SiteTransition.CrossFadeDuration`. The shipped graphics config
/// (player-data resource `graphics/data/mysekaigraphicsconfig`, JP 6.8.1) serialises
/// `_siteTransitionData._crossFadeDuration` as 0.5; the field initialiser's 0.25 is not
/// the shipped value. The runtime root carries no graphics-config file, so the value is
/// carried here like the other graphics-config constants.
pub(crate) const CANNON_SITE_FADE_SECONDS: f32 = 0.5;

/// The source site kinds `SiteMoveActionExecutor.GetActionState` switches on
/// (`MysekaiSiteType`: home_site 0, the three floors 1..3, the four harvest sites 4..7,
/// festival_garden 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SiteKind { Home, Room, Harvest, Delivery }

impl SiteKind {
    /// Master `siteType` names. A name outside the source enum is a data error.
    pub(crate) fn of(site_type: &str) -> Self {
        match site_type {
            "home_site" => Self::Home,
            "first_floor" | "second_floor" | "third_floor" => Self::Room,
            "grassland" | "shore" | "flower_garden" | "memorial_place" => Self::Harvest,
            "festival_garden" => Self::Delivery,
            other => panic!("site type {other} is not a MysekaiSiteType the site-move dispatch knows"),
        }
    }
}

/// Which `CrossFade` caller changes the environment, and so which duration it passes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum EnvironmentMove {
    /// Same site, another phenomenon (HomeSiteController / SiteLayoutEditor, 0.25 s).
    Phenomenon,
    /// `MoveSiteUseCannonActionState` (home, harvest and delivery sites among
    /// themselves).
    Cannon,
    /// `HomeToMyRoomActionState` / `MyRoomToHomeActionState`: FadeAnimationDuration 0.
    Door,
    /// `MyRoomToMyRoomActionState` calls no CrossFade; the floors share one environment
    /// site, so the product's rebuilt environment is taken at once.
    RoomToRoom,
    /// The product reloads the same site (a preview or a restored layout); the source
    /// dispatch returns no move state for a site to itself, so nothing fades.
    SameSite,
    /// A direct switch between a room and a harvest or delivery site. The source
    /// dispatch refuses it (it logs an error and returns no state); the product allows
    /// it as a shortcut and takes the duration of the source route's last leg: into an
    /// outdoor site that leg is the cannon, into a room it is the door.
    ProductShortcut { last_leg_cannon: bool },
}

impl EnvironmentMove {
    /// Classify a switch by the master `siteType` of the site left and the site entered.
    pub(crate) fn between(from_type: &str, to_type: &str) -> Self {
        use SiteKind::*;
        if from_type == to_type { return Self::SameSite; }
        match (SiteKind::of(from_type), SiteKind::of(to_type)) {
            (Home, Home) => Self::SameSite,
            (Home, Room) | (Room, Home) => Self::Door,
            (Room, Room) => Self::RoomToRoom,
            (Home | Harvest | Delivery, Home | Harvest | Delivery) => Self::Cannon,
            (Room, Harvest | Delivery) => Self::ProductShortcut { last_leg_cannon: true },
            (Harvest | Delivery, Room) => Self::ProductShortcut { last_leg_cannon: false },
        }
    }
    pub(crate) fn fade_seconds(self) -> f32 {
        match self {
            Self::Phenomenon => HOME_ENVIRONMENT_FADE_SECONDS,
            Self::Cannon | Self::ProductShortcut { last_leg_cannon: true } => CANNON_SITE_FADE_SECONDS,
            Self::Door | Self::RoomToRoom | Self::SameSite | Self::ProductShortcut { last_leg_cannon: false } => 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GlobalEffectIdentity {
    pub phenomenon_id: i32,
    pub renderer_type: i32,
}

/// A destination asset identity is deliberately stronger than global sky identity.
/// The site generation rejects completion from a detached/recreated controller.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnvironmentSelection {
    pub name: String,
    pub global_effect: GlobalEffectIdentity,
    pub site_id: u32,
    /// Master `siteType`: the site-move dispatch classifies moves by it.
    pub site_type: String,
    pub environment_site: String,
    pub site_generation: u64,
}

#[derive(Resource, Default, Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WeatherTransition {
    pub destination: Option<EnvironmentSelection>,
    /// Last completed RefreshShaderView/RefreshPostProcess, not the requested ID.
    pub committed: Option<EnvironmentSelection>,
    /// The frozen two EnvironmentLoadData values used by PrepareCrossFade.
    pub source: Option<EnvironmentSelection>,
    pub loaded: Option<EnvironmentSelection>,
    pub running: bool,
    pub progress: f32,
    pub request_serial: u64,
    pub site_generation: u64,
    loaded_serial: Option<u64>,
    committed_serial: Option<u64>,
    elapsed: f32,
    /// The duration the running fade was started with (0 ends it at its start).
    pub fade_seconds: f32,
    /// The CrossFade caller of the last started fade.
    pub movement: Option<EnvironmentMove>,
}

/// Stable user-visible state; animation progress stays out of the snapshot so
/// an open selector never rebuilds each frame or steals keyboard focus.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WeatherTransitionView {
    pub phase: &'static str,
    pub requested_id: Option<i32>,
    pub committed_id: Option<i32>,
    pub site_id: Option<u32>,
    pub request_serial: u64,
    pub committed_serial: Option<u64>,
}

impl WeatherTransition {
    pub fn presentation(&self) -> WeatherTransitionView {
        let committed = self.committed.as_ref().filter(|s| s.site_generation == self.site_generation);
        let phase = if self.destination.is_none() { "waiting" }
            else if self.running { "transitioning" }
            else if self.committed_serial == Some(self.request_serial) && committed == self.destination.as_ref() { "ready" }
            else { "loading" };
        WeatherTransitionView { phase,
            requested_id: self.destination.as_ref().map(|s| s.global_effect.phenomenon_id),
            committed_id: committed.map(|s| s.global_effect.phenomenon_id),
            site_id: self.destination.as_ref().map(|s| s.site_id),
            request_serial: self.request_serial,
            committed_serial: committed.and(self.committed_serial),
        }
    }

    pub fn committed_serial(&self) -> Option<u64> { self.committed_serial }

    pub fn request(&mut self, destination: EnvironmentSelection) {
        if self.destination.as_ref() == Some(&destination) { return; }
        // Cancellation of DoCrossFadeAsync writes endProgress before throwing;
        // CrossFadeCore's global/profile/timeline commit is not reached.
        self.running = false;
        self.progress = 1.0;
        self.source = self.loaded.clone();
        self.elapsed = 0.0;
        self.request_serial = self.request_serial.checked_add(1).expect("weather request serial overflow");
        self.destination = Some(destination);
    }

    pub fn invalidate_site(&mut self) {
        self.site_generation = self.site_generation.checked_add(1).expect("weather site generation overflow");
        self.request_serial = self.request_serial.checked_add(1).expect("weather request serial overflow");
        self.destination = None;
        self.running = false;
        self.progress = 1.0;
        self.source = self.loaded.clone();
        self.elapsed = 0.0;
    }

    /// True only on the start frame. Preparation does not create any effect.
    pub fn start_prepared(&mut self, prepared: &WeatherFxPrepared) -> bool {
        if prepared.request_serial != self.request_serial
            || self.destination.as_ref() != Some(&prepared.selection)
            || self.loaded_serial == Some(self.request_serial) { return false; }
        self.source = self.loaded.clone();
        self.loaded = self.destination.clone();
        self.loaded_serial = Some(self.request_serial);
        // A site switch bumps the generation, so the frozen source then belongs to the
        // site being left; the move between the two site kinds picks the duration.
        self.movement = match (&self.source, &self.loaded) {
            (Some(from), Some(to)) if from.site_generation != to.site_generation =>
                Some(EnvironmentMove::between(&from.site_type, &to.site_type)),
            (Some(_), Some(_)) => Some(EnvironmentMove::Phenomenon),
            _ => None,
        };
        self.fade_seconds = self.movement.map_or(0.0, EnvironmentMove::fade_seconds);
        self.running = self.source.is_some() && self.fade_seconds > 0.0;
        self.elapsed = 0.0;
        self.progress = if self.running { 0.0 } else { 1.0 };
        true
    }

    pub fn advance(&mut self, delta_seconds: f32) {
        assert!(delta_seconds.is_finite() && delta_seconds >= 0.0, "invalid weather clock delta");
        if !self.running { return; }
        // Use the supplied engine deltaTime; do not add a weather-only 50ms cap.
        self.elapsed += delta_seconds;
        self.progress = (self.elapsed / self.fade_seconds).clamp(0.0, 1.0);
        self.running = self.progress < 1.0;
    }

    pub fn can_start_site_fx(&self, selection: &EnvironmentSelection) -> bool {
        self.loaded_serial == Some(self.request_serial)
            && self.destination.as_ref() == Some(selection)
            && self.loaded.as_ref() == Some(selection)
    }
    pub fn can_commit_global_fx(&self, selection: &EnvironmentSelection) -> bool {
        self.can_start_site_fx(selection) && !self.running
    }

    /// Called after the real global FX installation, never as a readiness guess.
    pub fn commit(&mut self, effects: &WeatherGlobalFxCommitted) -> bool {
        if effects.0 != self.request_serial || self.committed_serial == Some(self.request_serial)
            || !self.loaded.as_ref().is_some_and(|s| self.can_commit_global_fx(s)) { return false; }
        self.committed = self.loaded.clone();
        self.committed_serial = Some(self.request_serial);
        true
    }
}

#[derive(Resource)]
pub(crate) struct WeatherFxPrepared {
    pub selection: EnvironmentSelection,
    pub request_serial: u64,
}
#[derive(Resource)]
pub(crate) struct WeatherGlobalFxCommitted(pub u64);
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct WeatherEnvironmentUpdate;

#[cfg(test)]
mod tests {
    use super::*;
    fn selection(id:i32, site:&str, generation:u64) -> EnvironmentSelection {
        EnvironmentSelection {name:format!("{id:03}"),global_effect:GlobalEffectIdentity {phenomenon_id:id,renderer_type:0},site_id:1,site_type:"home_site".into(),environment_site:site.into(),site_generation:generation}
    }
    fn prepare(phase:&WeatherTransition) -> WeatherFxPrepared {
        WeatherFxPrepared {selection:phase.destination.clone().unwrap(),request_serial:phase.request_serial}
    }
    fn installed(phase:&mut WeatherTransition, value:EnvironmentSelection) {
        phase.request(value);let ready=prepare(phase);assert!(phase.start_prepared(&ready));phase.advance(0.25);
        assert!(phase.commit(&WeatherGlobalFxCommitted(phase.request_serial)));
    }
    #[test]
    fn pending_assets_do_not_stop_or_start_the_visual_effects() {
        let mut p=WeatherTransition::default();let a=selection(1,"home",0);installed(&mut p,a.clone());
        let b=selection(8,"home",0);p.request(b.clone());
        assert_eq!(p.loaded,Some(a.clone()));assert_eq!(p.committed,Some(a));assert!(!p.can_start_site_fx(&b));
    }
    #[test]
    fn stale_weather_or_site_completion_cannot_release_a_replacement() {
        let mut p=WeatherTransition::default();p.request(selection(8,"home",0));let b=prepare(&p);
        p.request(selection(9,"home",0));assert!(!p.start_prepared(&b));
        p.request(selection(8,"beach",0));assert!(!p.start_prepared(&b));
        p.invalidate_site();p.request(selection(8,"home",p.site_generation));assert!(!p.start_prepared(&b));
    }
    #[test]
    fn site_fx_precedes_the_global_and_camera_commit() {
        let mut p=WeatherTransition::default();installed(&mut p,selection(1,"home",0));
        let b=selection(8,"home",0);p.request(b.clone());let ready=prepare(&p);p.start_prepared(&ready);
        assert!(p.can_start_site_fx(&b));assert!(!p.can_commit_global_fx(&b));
        p.advance(0.24);assert!(!p.can_commit_global_fx(&b));p.advance(0.02);assert!(p.can_commit_global_fx(&b));
        assert_ne!(p.committed,Some(b.clone()));assert!(p.commit(&WeatherGlobalFxCommitted(p.request_serial)));assert_eq!(p.committed,Some(b));
    }
    #[test]
    fn cancellation_ends_visual_fade_without_committing_profile_or_effects() {
        let mut p=WeatherTransition::default();let a=selection(1,"home",0);installed(&mut p,a.clone());
        let b=selection(8,"home",0);p.request(b.clone());let ready=prepare(&p);p.start_prepared(&ready);p.advance(0.1);
        let obsolete=p.request_serial;p.request(selection(9,"home",0));
        assert_eq!(p.progress,1.0);assert_eq!(p.loaded,Some(b));assert_eq!(p.committed,Some(a));
        assert!(!p.commit(&WeatherGlobalFxCommitted(obsolete)));
    }
    #[test]
    fn renderer_identity_is_not_destination_asset_identity() {
        let a=selection(1,"home",0);let mut b=selection(1,"beach",0);
        assert_ne!(a,b);assert_eq!(a.global_effect,b.global_effect);
        b.global_effect.renderer_type=1;assert_ne!(a.global_effect,b.global_effect);
    }
    #[test]
    fn frame_delta_is_not_artificially_capped_to_fifty_milliseconds() {
        let mut p=WeatherTransition::default();installed(&mut p,selection(1,"home",0));
        p.request(selection(8,"home",0));let ready=prepare(&p);p.start_prepared(&ready);p.advance(0.2);
        assert!((p.progress-0.8).abs()<1e-6);
    }
    #[test]
    fn ui_never_reports_a_request_as_the_completed_environment() {
        let mut p = WeatherTransition::default();
        assert_eq!(p.presentation().phase, "waiting");
        installed(&mut p, selection(1, "home", 0));
        let stable = p.presentation();
        assert_eq!(stable.phase, "ready");
        p.request(selection(8, "home", 0));
        assert_eq!(p.presentation().requested_id, Some(8));
        assert_eq!(p.presentation().committed_id, Some(1));
        assert_eq!(p.presentation().phase, "loading");
        p.start_prepared(&prepare(&p));
        assert_eq!(p.presentation().phase, "transitioning");
        let start = p.presentation();p.advance(0.1);
        assert_eq!(start, p.presentation(), "per-frame progress must not replace the UI tree");
        p.request(selection(9, "home", 0));
        assert_eq!(p.presentation().committed_id, Some(1));
        assert_eq!(p.presentation().requested_id, Some(9));
        p.start_prepared(&prepare(&p));p.advance(0.3);
        assert_eq!(p.presentation().phase, "loading", "render completion alone is not the source commit");
        assert!(p.commit(&WeatherGlobalFxCommitted(p.request_serial)));
        assert_eq!(p.presentation().committed_id, Some(9));
        assert_eq!(p.presentation().phase, "ready");
        p.invalidate_site();
        assert_eq!(p.presentation().committed_id, None);
        assert_eq!(p.presentation().phase, "waiting");
    }

}
