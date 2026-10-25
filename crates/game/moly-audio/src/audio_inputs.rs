//! State that the world and the game publish for audio: what the audio systems
//! read about sites, talks and music settings. The publishing systems live with
//! the domains that own the state.

use bevy::prelude::*;
use moly_protocol::json::music_settings_value as rows_value;
use moly_protocol::rows::MusicPlaySetting;
use serde_json::Value;
use std::collections::BTreeMap;

/// 一条 voice 行的说话者指称：voice 步的 `who`（角色变体 id）或
/// fixture_voice 步的 `fixture`（家具 id）。partvoice 的包随说话者变体
/// （真源按参与者逐个载包），不随 cue——cue 名里的段不编码变体归属。
#[derive(Clone, Copy, Debug)]
pub enum VoiceWho {
    /// voice 步：characterUnitId（名册变体 id）。
    Participant(i32),
    /// fixture_voice 步：mysekaiFixtureId（家具角色的单参包，不拼 unit）。
    Fixture(i32),
}

/// A future authored voice command. Preparing one only requests the exact raw
/// audio asset; it never creates an AudioPlayer, changes the single voice bus,
/// or advances dialogue timing.
#[derive(Clone, Debug)]
pub struct VoicePrefetch {
    pub cue: String,
    pub who: Option<VoiceWho>,
}

impl VoicePrefetch {
    pub fn new(cue: String, who: Option<VoiceWho>) -> Self {
        Self { cue, who }
    }
}

/// Which voice commands the playing conversations will need next. The talk
/// domains publish them; audio only loads the raw assets, it never advances a
/// script from this.
#[derive(Resource, Default)]
pub struct TalkVoicePrefetch {
    /// The exact future voice commands of every playing conversation.
    pub requests: Vec<VoicePrefetch>,
}

/// `GameStateManager.HasContext(MysekaiContextType.Birthday)`: the audio
/// domain's own view of it, set while a birthday cut-scene holds the state.
#[derive(Resource, Clone, Copy, Debug)]
pub struct BirthdayVoice {
    pub party: i64,
}

/// The tutorial manager's active flag. The world publishes it; voice-bank
/// selection gives this priority over the birthday context.
#[derive(Resource, Clone, Copy, Debug)]
pub struct TutorialVoice;

/// `MysekaiBGMManager.PlayBGMAsync` for the destination has not run yet: the
/// current music keeps playing across a site move.
#[derive(Resource)]
pub struct BgmHold;

/// 当前现象档名——天气域对外的一面：音频 BGM（按站点 × 档选曲）与小地图
/// 天气钮（图标与文本）都读它，不进天气域运行态的私有下标。
#[derive(Resource)]
pub struct CurrentPhenomenon(pub String);

impl Default for CurrentPhenomenon {
    fn default() -> Self {
        Self("001_sunny".to_string())
    }
}

/// The client's copy, set by responses only.
#[derive(Resource, Debug, Clone, Default)]
pub struct ClientMusicPlaySettings {
    per_site: BTreeMap<u32, MusicPlaySetting>,
    /// Responses that carried the section.
    pub revision: u64,
}

#[allow(dead_code)] // Read by the BGM owner's seam.
impl ClientMusicPlaySettings {
    /// The setting of a site: (record id, vocal id).
    pub fn setting(&self, site_id: u32) -> Option<(i64, i64)> {
        self.per_site.get(&site_id).map(|row| {
            (
                i64::from(row.mysekai_music_record_id),
                i64::from(row.music_vocal_id),
            )
        })
    }

    /// The whole row of a site.
    pub fn row(&self, site_id: u32) -> Option<&MusicPlaySetting> {
        self.per_site.get(&site_id)
    }

    /// A response's rows replace the copy (a row with a negative site id
    /// names no site and is dropped).
    pub fn apply(&mut self, rows: Vec<MusicPlaySetting>) {
        self.per_site = rows
            .into_iter()
            .filter_map(|row| {
                u32::try_from(row.mysekai_site_id)
                    .ok()
                    .map(|site| (site, row))
            })
            .collect();
        self.revision += 1;
    }

    pub fn view(&self) -> Value {
        rows_value(&self.per_site.values().copied().collect::<Vec<_>>())
    }
}

