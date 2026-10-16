//! The collect-item cells: `Initialize`'s five instances of the layer's
//! `collectItemCellPrefab` composed into the notice document, and
//! `HarvestCollectItemNoticeCell.Setup` drawn on them.
//!
//! `Setup(data)`, read from the compiled method: `_itemNameText.text =
//! ItemName`, `_newObject.SetActive(IsNew)`, `SetCount(GetCount)` (the count
//! text is `"×" + count.ToString()`), `SetupLimit(IsLimit)`,
//! `SetupEffect(IsRare)`, `LoadItemImage(data).Forget()`.
//! - `SetupLimit`: the limit text's object active when limited, the name's
//!   when not, and the count text's colour set to palette entry `text_rd`
//!   (limited) or `text_pk`. The count text also carries a
//!   `GraphicColorSynchronizer` on `text_pk`; its `OnEnable` subscribes to the
//!   entry and applies the entry's value. `Setup` runs while the cell is
//!   asleep and `Wakeup` follows it, so the synchronizer writes `text_pk`
//!   over the limited colour every time the cell shows: the shown count is
//!   always `text_pk`, which is the text's serialized colour. No colour is
//!   written here for that reason.
//! - `SetupEffect`: `_rareEffect.SetActive(false)`; when rare,
//!   `SystemUtility.DelayCall(SingletonManager, () => _rareEffect.SetActive(
//!   true), 0.15)`, a coroutine that counts `Time.deltaTime` while below the
//!   delay (the first count in the calling frame). Named gap: the rare
//!   effect's content is one `UIParticle` whose component is not decoded, so
//!   the effect's object is switched and nothing is drawn under it.
//! - `LoadItemImage`: with both names set, `UITextureLoader.LoadAsync(bundle,
//!   resource)`: the raw image is disabled and the loading object shown
//!   during the load; when it ends the loading object is hidden and the raw
//!   image enabled. Here the texture is resolved by the client's load path
//!   ([`crate::item_icon`]) and bound at `Setup`, and the loading object
//!   stays hidden (the load phase has no duration here). A load path with no texture is refused by
//!   name and the image stays hidden: the source's failed load would show the
//!   raw image with no texture.
//! - The new mark (`_newObject`): its `ColorFader` has no target, no colour
//!   table and neither play-on-awake nor play-on-enable, so its `Awake` only
//!   activates its own object (already active) and nothing fades. Each letter
//!   has a `TweenPosition` (play on enable, endless restart loops, the
//!   letter's animation curve as ease, its own delay): on every wake of a new
//!   cell the letter jumps to `from` and loops towards `to`; the tween dies
//!   with the object (`dontKillIfDisable` off).

use bevy::math::{Vec2, Vec3};
use moly_assets::ui_layout::{UiInstance, UiPrefab};
use moly_law::particle::{curve::eval_curve, value::CurveKey};
use serde_json::Value;

use super::slide::PARALLEL;

pub(crate) const PRESENTER: &str = "Sekai.Mysekai.ScreenLayerMysekaiNotice";
const CELL: &str = "Sekai.Mysekai.HarvestCollectItemNoticeCell";
const LOADER: &str = "Sekai.UI.UITextureLoader";
const TWEEN_POSITION: &str = "Sekai.TweenPosition";
/// `SetupEffect`'s `DelayCall` delay.
pub(crate) const RARE_DELAY: f32 = 0.15;

fn pointer(value: &Value, what: &str) -> Result<i64, String> {
    let pair = value
        .as_array()
        .filter(|pair| pair.len() == 2)
        .ok_or_else(|| format!("{what} is not a serialized reference"))?;
    if pair[0].as_i64() != Some(0) {
        return Err(format!("{what} points outside the prefab"));
    }
    pair[1]
        .as_i64()
        .filter(|id| *id != 0)
        .ok_or_else(|| format!("{what} is null"))
}

fn component<'a>(doc: &'a UiPrefab, node: usize, class: &str) -> Result<&'a Value, String> {
    doc.nodes[node]
        .components
        .iter()
        .find(|c| c.class == class)
        .map(|c| &c.fields)
        .ok_or_else(|| format!("{} has no {class}", doc.nodes[node].path))
}

/// The GameObject selector of the node an object id names.
fn node_selector(doc: &UiPrefab, id: i64) -> Result<(usize, String), String> {
    let index = doc.find(&format!("@{id}"))?;
    Ok((index, format!("@{}", doc.nodes[index].game_object_id)))
}

/// The five cells composed into a copy of the notice document, named as
/// `Initialize` names them. Returns the composed document, the instances and
/// the cells' anchored x and height.
pub(crate) fn compose(doc: &UiPrefab) -> Result<(UiPrefab, Vec<UiInstance>, f32, f32), String> {
    let presenter = doc
        .nodes
        .iter()
        .position(|node| node.components.iter().any(|c| c.class == PRESENTER))
        .ok_or("no ScreenLayerMysekaiNotice presenter")?;
    let fields = component(doc, presenter, PRESENTER)?;
    let root = pointer(&fields["collectItemRoot"], "collectItemRoot")?;
    let prefab = pointer(&fields["collectItemCellPrefab"], "collectItemCellPrefab")?;
    let template = doc.find(&format!("@{prefab}"))?;
    let rect = &doc.nodes[template].rect;
    // `rect.height` of the cell: with both y anchors equal it is sizeDelta.y.
    if rect.anchors_min[1] != rect.anchors_max[1] {
        return Err(format!(
            "the cell's y anchors differ ({} / {}): its rect height depends on the root's",
            rect.anchors_min[1], rect.anchors_max[1]
        ));
    }
    let (x, height) = (rect.anchored_position[0], rect.size_delta[1]);
    let name = doc.nodes[template].name.clone();
    let names: Vec<String> = (0..PARALLEL).map(|i| format!("{name}{i}")).collect();
    let mut composed = doc.clone();
    let instances = composed.instantiate_children(
        &format!("@{root}"),
        doc,
        &format!("@{prefab}"),
        names.iter().map(String::as_str),
    )?;
    Ok((composed, instances, x, height))
}

/// One `TweenPosition` as `TweenBase.SetTweenOptionCommon` configures the
/// letters': ease = the curve, `SetDelay(delay)`, `SetLoops(-1, Restart)`.
#[derive(Clone, Debug)]
pub(crate) struct LetterLaw {
    pub(crate) from: Vec3,
    pub(crate) to: Vec3,
    pub(crate) delay: f32,
    pub(crate) duration: f32,
    pub(crate) keys: Vec<CurveKey>,
}

/// The DOTween tweener of one letter.
#[derive(Clone, Debug)]
pub(crate) struct LetterTween {
    law: LetterLaw,
    delay_complete: bool,
    elapsed_delay: f32,
    startup_done: bool,
    start: Vec3,
    change: Vec3,
    position: f32,
    completed_loops: i32,
    value: Vec3,
}

impl LetterTween {
    /// `TweenPosition.PlayCore(Forward)`: the position is set to `from`, then
    /// the tweener is created.
    pub(crate) fn play(law: LetterLaw) -> Self {
        Self {
            delay_complete: law.delay <= 0.0,
            elapsed_delay: 0.0,
            startup_done: false,
            start: law.from,
            change: Vec3::ZERO,
            position: 0.0,
            completed_loops: 0,
            value: law.from,
            law,
        }
    }

    pub(crate) fn value(&self) -> Vec3 {
        self.value
    }

    /// One `TweenManager.Update` step (Normal update, time scale 1, endless
    /// Restart loops). Returns whether a value was applied.
    pub(crate) fn update(&mut self, dt: f32) -> bool {
        let mut delta = dt;
        if !self.delay_complete {
            let elapsed = self.elapsed_delay + delta;
            if self.law.delay < elapsed {
                self.elapsed_delay = self.law.delay;
                self.delay_complete = true;
                delta = elapsed - self.law.delay;
            } else {
                self.elapsed_delay = elapsed;
                delta = 0.0;
            }
            if delta <= 0.0 {
                return false;
            }
        }
        if !self.startup_done {
            self.startup_done = true;
            self.start = self.value;
            self.change = self.law.to - self.start;
        }
        let duration = self.law.duration;
        let mut to_position = delta + self.position;
        let mut loops = self.completed_loops;
        while duration <= to_position {
            to_position -= duration;
            loops += 1;
        }
        if duration <= self.position {
            loops -= 1;
        }
        self.completed_loops = loops;
        self.position = if to_position <= duration {
            if to_position > 0.0 {
                to_position
            } else if loops > 0 {
                duration
            } else {
                0.0
            }
        } else {
            duration
        };
        let last = self.law.keys.last().map_or(0.0, |key| key.time);
        let ease = eval_curve(&self.law.keys, (self.position / duration) * last);
        self.value = self.start + self.change * ease;
        true
    }
}

/// One letter of the new mark.
#[derive(Clone, Debug)]
pub(crate) struct Letter {
    pub(crate) selector: String,
    pub(crate) law: LetterLaw,
}

/// The selectors and laws of one composed cell.
#[derive(Clone, Debug)]
pub(crate) struct CellBinding {
    pub(crate) root: String,
    pub(crate) y: f32,
    pub(crate) name: String,
    pub(crate) count: String,
    pub(crate) limit: String,
    pub(crate) new_mark: String,
    pub(crate) rare: String,
    pub(crate) icon: String,
    pub(crate) loading: String,
    pub(crate) letters: Vec<Letter>,
}

fn number(value: &Value, what: &str) -> Result<f32, String> {
    value
        .as_f64()
        .map(|v| v as f32)
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("{what} is not a number"))
}

fn vec3(value: &Value, what: &str) -> Result<Vec3, String> {
    let a = value
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or_else(|| format!("{what} is not a Vector3"))?;
    Ok(Vec3::new(
        number(&a[0], what)?,
        number(&a[1], what)?,
        number(&a[2], what)?,
    ))
}

/// A letter's `TweenPosition`, refused unless it is the form ported here.
fn letter_law(doc: &UiPrefab, node: usize) -> Result<LetterLaw, String> {
    let t = component(doc, node, TWEEN_POSITION)?;
    let path = &doc.nodes[node].path;
    for (field, wanted) in [
        ("behaviour", 0),
        ("componentType", 0),
        ("direction", 0),
        ("startTiming", 2),
        ("loopType", 1),
        ("loopCount", 0),
    ] {
        if t[field].as_i64() != Some(wanted) {
            return Err(format!(
                "{path}: TweenPosition {field} is {}, the port covers {wanted}",
                t[field]
            ));
        }
    }
    for field in ["dontKillIfDisable", "isReflesh", "syncGameTime"] {
        if t[field].as_bool() != Some(false) {
            return Err(format!(
                "{path}: TweenPosition {field} is {}, the port covers false",
                t[field]
            ));
        }
    }
    let curve = &t["curve"];
    if curve["preWrap"].as_i64() != Some(2) || curve["postWrap"].as_i64() != Some(2) {
        return Err(format!(
            "{path}: the ease curve does not clamp at both ends"
        ));
    }
    let keys = curve["keys"]
        .as_array()
        .ok_or_else(|| format!("{path}: ease curve without keys"))?
        .iter()
        .map(|key| {
            Ok(CurveKey {
                time: number(&key["time"], "key time")?,
                value: number(&key["value"], "key value")?,
                in_slope: number(&key["inTangent"], "key inTangent")?,
                out_slope: number(&key["outTangent"], "key outTangent")?,
                weighted_mode: u8::try_from(
                    key["weightedMode"].as_u64().ok_or("key weightedMode")?,
                )
                .map_err(|_| "key weightedMode out of range".to_owned())?,
                in_weight: number(&key["inWeight"], "key inWeight")?,
                out_weight: number(&key["outWeight"], "key outWeight")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if keys.is_empty() || keys.windows(2).any(|pair| pair[1].time < pair[0].time) {
        return Err(format!(
            "{path}: the ease curve keys are empty or unordered"
        ));
    }
    let duration = number(&t["duration"], "duration")?;
    if duration <= 0.0 {
        return Err(format!("{path}: tween duration {duration}"));
    }
    // The tween moves `localPosition`; it is the anchored position here only
    // with the anchors on the parent's pivot.
    let rect = &doc.nodes[node].rect;
    let parent = doc
        .parent(node)
        .ok_or_else(|| format!("{path}: a letter without a parent"))?;
    if rect.anchors_min != rect.anchors_max || rect.anchors_min != doc.nodes[parent].rect.pivot {
        return Err(format!("{path}: its anchors are not on the parent's pivot"));
    }
    Ok(LetterLaw {
        from: vec3(&t["from"], "from")?,
        to: vec3(&t["to"], "to")?,
        delay: number(&t["delay"], "delay")?,
        duration,
        keys,
    })
}

/// Bind one composed cell through its own cell component's references.
pub(crate) fn bind(
    doc: &UiPrefab,
    instance: &UiInstance,
    index: usize,
    height: f32,
) -> Result<CellBinding, String> {
    let (root_index, root) = node_selector(doc, instance.root_transform_id)?;
    let fields = component(doc, root_index, CELL)?;
    let reference = |name: &str| pointer(&fields[name], name);
    let (_, name) = node_selector(doc, reference("_itemNameText")?)?;
    let (_, count) = node_selector(doc, reference("_itemGetCountText")?)?;
    let (_, limit) = node_selector(doc, reference("_itemLimitText")?)?;
    let (new_index, new_mark) = node_selector(doc, reference("_newObject")?)?;
    let (_, rare) = node_selector(doc, reference("_rareEffect")?)?;
    let (icon_index, icon) = node_selector(doc, reference("_iconLoader")?)?;
    let loader = component(doc, icon_index, LOADER)?;
    let (_, loading) = node_selector(doc, pointer(&loader["loadingObject"], "loadingObject")?)?;
    let graphic = pointer(&loader["targetGraphic"], "targetGraphic")?;
    if !doc.nodes[icon_index]
        .components
        .iter()
        .any(|c| c.path_id == graphic)
    {
        return Err("the loader's target graphic is not on the loader's object".into());
    }
    let mut letters = Vec::new();
    for (node, _) in doc
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, _)| doc.parent(*i) == Some(new_index))
    {
        if doc.nodes[node]
            .components
            .iter()
            .any(|c| c.class == TWEEN_POSITION)
        {
            letters.push(Letter {
                selector: format!("@{}", doc.nodes[node].game_object_id),
                law: letter_law(doc, node)?,
            });
        }
    }
    Ok(CellBinding {
        root,
        y: -height * index as f32,
        name,
        count,
        limit,
        new_mark,
        rare,
        icon,
        loading,
        letters,
    })
}

/// `SetCount`: `"×" + current.ToString()`.
pub(crate) fn count_text(count: i32) -> String {
    format!("\u{00D7}{count}")
}

/// The anchored position a letter's tween value writes.
pub(crate) fn letter_position(value: Vec3) -> Vec2 {
    value.truncate()
}
