//! The game's `GraphicButtonTapEffect`, the view interaction a
//! `CustomButton` plays on press and release, and the colour tween it runs.
//!
//! The effect holds two palette colours: the default colour and the effect
//! colour, each the active theme value of a palette entry (`_defaultColorPalette`,
//! `_effectColorPalette`); with `_useCustomAlpha` the stored colour takes the
//! serialized custom alpha instead of the palette alpha. Awake assigns the
//! default colour to the effect graphic when `_refreshColorWhenAwake` is set
//! (killing any fade first). A press kills the running fades and fades the
//! graphic to the effect colour over the base class' fade-in time; a release
//! kills them and fades back to the default colour over the fade-out time.
//!
//! A fade is DOTween's `DOColor` on the Graphic: the tween reads the graphic's
//! colour when it starts (its first update), keeps `change = end - start`
//! per channel, and on every update writes `start + change * eased` for all
//! four channels, with `eased` the default ease of the tween settings at the
//! tween's position; the position is the running sum of the update deltas,
//! capped at the duration on the completing update, after which the tween
//! is killed. The Graphic colour reaches the vertices as `Color32`.

use super::unity_math;

/// `ButtonViewInteractionBase.FADE_IN_TIME`: the press fade.
pub const FADE_IN_TIME: f32 = 0.1;
/// `ButtonViewInteractionBase.FADE_OUT_TIME`: the release fade.
pub const FADE_OUT_TIME: f32 = 0.4;

/// The serialized `GraphicButtonTapEffect` fields the colours come from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TapEffectConfig {
    pub default_palette: usize,
    pub effect_palette: usize,
    pub use_custom_alpha: bool,
    pub custom_default_alpha: f32,
    pub custom_effect_alpha: f32,
    pub refresh_color_when_awake: bool,
}

/// The stored default and effect colours (`_defaultColor`, `_effectColor`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TapEffectColors {
    pub default: [f32; 4],
    pub effect: [f32; 4],
}

impl TapEffectConfig {
    /// `RefreshColor`: `SetDefaultColor(GetColor(default))` and
    /// `SetEffectColor(GetColor(effect))`, where each setter replaces the
    /// alpha by its custom alpha when `_useCustomAlpha` is set. `palette`
    /// answers `PaletteUtility.GetColor` for an entry (None: the palette has
    /// no such entry).
    pub fn colors(&self, palette: impl Fn(usize) -> Option<[f32; 4]>) -> Result<TapEffectColors, String> {
        let entry = |index: usize| palette(index).ok_or_else(|| format!("palette has no colour entry {index}"));
        let mut default = entry(self.default_palette)?;
        let mut effect = entry(self.effect_palette)?;
        if self.use_custom_alpha {
            default[3] = self.custom_default_alpha;
            effect[3] = self.custom_effect_alpha;
        }
        Ok(TapEffectColors { default, effect })
    }
}

/// `EaseManager.Evaluate` for OutQuad at a position already capped to the
/// duration, in the compiled order `(t / d - 2) * -(t / d)`.
pub fn out_quad(position: f32, duration: f32) -> f32 {
    let t = position / duration;
    (t - 2.0) * -t
}

/// One `DOColor` tween.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorTween {
    end: [f32; 4],
    duration: f32,
    /// Start and change, set by the first update.
    started: Option<([f32; 4], [f32; 4])>,
    /// Running sum of the update deltas.
    position: f32,
}

impl ColorTween {
    pub fn new(end: [f32; 4], duration: f32) -> Self {
        Self { end, duration, started: None, position: 0.0 }
    }

    /// One tween-manager update with `delta`: `current` is the graphic's
    /// colour, read only by the first update. Returns the colour the setter
    /// writes and whether the tween completed (and is killed).
    pub fn update(&mut self, current: [f32; 4], delta: f32) -> ([f32; 4], bool) {
        let (start, change) = *self.started.get_or_insert_with(|| {
            let change = [0, 1, 2, 3].map(|k| self.end[k] - current[k]);
            (current, change)
        });
        self.position += delta;
        let complete = self.duration <= self.position;
        let position = if complete { self.duration } else { self.position };
        let eased = out_quad(position, self.duration);
        ([0, 1, 2, 3].map(|k| start[k] + change[k] * eased), complete)
    }
}

/// The effect's running fade (`_effectTweener`; the icon graphic's tween is
/// the same fade on a second graphic and is not held here).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TapEffectState {
    pub fade: Option<ColorTween>,
}

impl TapEffectState {
    /// `Awake`: the default colour to assign, when the effect refreshes on awake.
    pub fn awake(&mut self, config: &TapEffectConfig, colors: &TapEffectColors) -> Option<[f32; 4]> {
        if !config.refresh_color_when_awake {
            return None;
        }
        self.fade = None;
        Some(colors.default)
    }

    /// `OnPressed`: kill the fades, fade to the effect colour.
    pub fn on_pressed(&mut self, colors: &TapEffectColors) {
        self.fade = Some(ColorTween::new(colors.effect, FADE_IN_TIME));
    }

    /// `OnReleased`: kill the fades, fade to the default colour.
    pub fn on_released(&mut self, colors: &TapEffectColors) {
        self.fade = Some(ColorTween::new(colors.default, FADE_OUT_TIME));
    }

    /// One tween-manager update: the colour written, if a fade ran.
    pub fn update(&mut self, current: [f32; 4], delta: f32) -> Option<[f32; 4]> {
        let fade = self.fade.as_mut()?;
        let (color, complete) = fade.update(current, delta);
        if complete {
            self.fade = None;
        }
        Some(color)
    }
}

/// The colour a Graphic's vertices carry for a Graphic colour: its `Color32`.
pub fn vertex_color(color: [f32; 4]) -> [f32; 4] {
    unity_math::color32(color).map(|channel| f32::from(channel) / 255.0)
}

#[cfg(test)]
mod source_compare {
    use super::*;

    /// Runs the ease and the colour tween on cases given as text lines
    /// (`ease TIME DURATION`, or `tween START(4) END(4) DURATION DELTA...`, all
    /// f32 bits in hex) and writes one line per ease case (the eased bits) and
    /// one line per tween step (`r g b a complete`, until the step that
    /// completes), for comparison with the source methods executed on the same
    /// cases.
    #[test]
    #[ignore = "research instrument: needs MOLY_TWEEN_COMPARE_IN and MOLY_TWEEN_COMPARE_OUT"]
    fn tween_for_source_comparison() {
        let input = std::env::var("MOLY_TWEEN_COMPARE_IN").expect("MOLY_TWEEN_COMPARE_IN");
        let output = std::env::var("MOLY_TWEEN_COMPARE_OUT").expect("MOLY_TWEEN_COMPARE_OUT");
        let text = std::fs::read_to_string(&input).expect("case file");
        let bits = |w: &str| f32::from_bits(u32::from_str_radix(w.trim_start_matches("0x"), 16).expect("hex bits"));
        let hex = |v: f32| format!("{:#010x}", v.to_bits());
        let mut out = String::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let words: Vec<&str> = line.split_whitespace().collect();
            match words[0] {
                "ease" => {
                    assert_eq!(words.len(), 3, "case line: {line}");
                    out.push_str(&format!("ease {}\n", hex(out_quad(bits(words[1]), bits(words[2])))));
                }
                "tween" => {
                    assert!(words.len() >= 11, "case line: {line}");
                    let start = [1, 2, 3, 4].map(|k| bits(words[k]));
                    let end = [5, 6, 7, 8].map(|k| bits(words[k]));
                    let mut tween = ColorTween::new(end, bits(words[9]));
                    let mut current = start;
                    for delta in &words[10..] {
                        let (color, complete) = tween.update(current, bits(delta));
                        current = color;
                        out.push_str(&format!(
                            "step {} {} {} {} {}\n", hex(color[0]), hex(color[1]), hex(color[2]), hex(color[3]), complete
                        ));
                        if complete {
                            break;
                        }
                    }
                }
                other => panic!("case kind {other}"),
            }
        }
        std::fs::write(&output, out).expect("write report");
    }
}
