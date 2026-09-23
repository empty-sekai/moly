//! Ordered single-pointer gesture processing, shared by native and wasm.
//!
//! Drag threshold is min(screen width, height)/100, initialized once when a
//! drawable window exists. Long-touch uses the serialized 0.25s threshold;
//! consecutive taps use the 0.3s constructor constant.
//!
//! WindowEvent preserves cursor/button/touch ordering across slow frames.
//! Cursor positions and deltas are logical screen pixels, never raw mouse
//! device counts. A complete down/up pair is not reduced to a final held flag.
//! Source gesture classification and publication remain shared by both inputs.
//!
//! Ownership is captured at pointer-down using source UI geometry or the
//! current modal layer. A captured drag cannot leak into the scene camera;
//! losing focus or canceling a finger relinquishes the pointer without a click.
//!
//! Multi-finger pinch gestures remain unimplemented. Mouse wheel zoom is the
//! separate desktop adaptation in camera; it is not a second gesture engine.

use crate::joystick::{self, JoystickState};
use bevy::input::touch::TouchPhase;
use bevy::prelude::*;

/// 手势种类的闭集（真源 GestureType 的单指子集；数值对应真源
/// TAP=1、DOUBLE_TAP=4、DRAG=16、LONG_TOUCH=128）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureKind {
    Tap,
    DoubleTap,
    Drag,
    LongTouch,
}

impl GestureKind {
    /// tap 族判定（真源对话引擎的门是位掩码 0x85 = 1|4|128）。
    pub fn is_tap_family(self) -> bool {
        matches!(self, GestureKind::Tap | GestureKind::DoubleTap | GestureKind::LongTouch)
    }
}

/// 事件态（真源 InputState：BEGAN=0、UPDATE=1、END=2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureState {
    Began,
    Moved,
    End,
}

/// 手势事件（真源 GestureEventData 的单指投影）：种类 + 态 + 触点
/// 屏位（逻辑像素）+ 当帧增量。单指族的位 = 触点当前位（源各发布器
/// 对 1/4/16/128 都取首触点当前位覆写）；增量只有 DRAG 事件携带。
#[derive(Debug, Clone, Copy, Message)]
pub struct GestureEvent {
    pub kind: GestureKind,
    pub state: GestureState,
    pub position: Vec2,
    pub delta: Vec2,
    /// Ownership at pointer-down, retained through the entire drag/release.
    pub ui_owned: bool,
}

/// 单触点的跟踪面（真源 TouchData 的字段子集：StartPosition/
/// CurrentPosition/Delta/touchingTime）+ 输入源标记。
struct Touch {
    start: Vec2,
    current: Vec2,
    delta: Vec2,
    touching_time: f32,
    source: PointerSource,
    ui_owned: bool,
}

/// 在跟指针的来源：鼠标或某根手指（三处读法按源分路，见模块注释）。
#[derive(Clone, Copy, PartialEq)]
enum PointerSource {
    Mouse,
    Finger(u64),
}

/// 手势层状态（真源 GestureLayer 字段面的单指子集）。
#[derive(Resource, Default)]
pub(crate) struct GestureLayerState {
    cursor: Option<Vec2>,
    /// 拖拽阈值（源 Start 算一次后冻结；None = 还没算到窗口尺寸）。
    drag_threshold: Option<f32>,
    touch: Option<Touch>,
    /// 在跟的手势（源 trackingGesture；0 = 无）。
    tracking: Option<GestureKind>,
    /// 上一次收场的手势与时刻（源 lastGesture/lastGesturedTime，真时钟）。
    last_gesture: Option<GestureKind>,
    last_gestured_at: f32,
    /// 位移累计量（源 dragDelta；pinchDelta 单指恒 0 不设）。
    drag_delta: f32,
}

/// 源 Start：拖拽阈值 = min(屏宽, 屏高)/100。Unity 的 Screen 是整个
/// 游戏画面，本栈最接近的对应物是主窗口逻辑尺寸。
const DRAG_THRESHOLD_DIVISOR: f32 = 100.0;

/// 长按阈值（源序列化字段 LongTouchThreshold 的直读值）。
const LONG_TOUCH_THRESHOLD: f32 = 0.25;

/// 连击窗（源构造函数写死的 0.3，非序列化）。
const CONSECUTIVE_TAP_THRESHOLD: f32 = 0.3;

/// Consume the window's ordered pointer stream, not its final per-frame button
/// snapshot. Down/up can both occur between two rendered frames.
pub(crate) fn advance(
    mut layer: ResMut<GestureLayerState>,
    mut events: MessageWriter<GestureEvent>,
    mut input: MessageReader<bevy::window::WindowEvent>,
    joystick: Res<JoystickState>,
    windows: Query<(Entity, &Window), With<bevy::window::PrimaryWindow>>,
    time: Res<Time>,
    real: Res<Time<Real>>,
    settings_panel: Res<crate::game_settings::SettingsPanel>,
    ui: crate::ui_layout::PointerUi,
) {
    use bevy::{input::ButtonState, window::WindowEvent};
    let Ok((window_entity, window)) = windows.single() else {
        input.clear();
        return;
    };
    let now = real.elapsed_secs();
    if settings_panel.blocks_world_input() {
        cancel(&mut layer, &mut events);
        layer.touch = None;
        layer.cursor = window.cursor_position();
        input.clear();
        return;
    }
    let size = Vec2::new(window.width(), window.height());
    if !size.is_finite() || size.min_element() <= 0. {
        cancel(&mut layer, &mut events);
        layer.touch = None;
        input.clear();
        return;
    }
    let threshold = *layer.drag_threshold.get_or_insert_with(|| {
        let value = size.min_element() / DRAG_THRESHOLD_DIVISOR;
        info!("[gesture] screen drag threshold {value:.1}px");
        value
    });
    for event in input.read() {
        match event {
            WindowEvent::CursorMoved(moved) if moved.window == window_entity => {
                layer.cursor = Some(moved.position);
                move_pointer(&mut layer, &mut events, PointerSource::Mouse, moved.position, threshold);
            }
            WindowEvent::MouseButtonInput(button)
                if button.window == window_entity && button.button == MouseButton::Left =>
            {
                let position = layer.cursor.or_else(|| window.cursor_position());
                if button.state == ButtonState::Pressed {
                    if let Some(position) = position {
                        let ui_owned = ui.captures(position, size);
                        press_pointer(&mut layer, &mut events, PointerSource::Mouse, position, now, ui_owned);
                    }
                } else {
                    release_pointer(&mut layer, &mut events, PointerSource::Mouse, position, now);
                }
            }
            WindowEvent::TouchInput(touch) if touch.window == window_entity => {
                let source = PointerSource::Finger(touch.id);
                match touch.phase {
                    TouchPhase::Started => {
                        if joystick.captured == Some(touch.id)
                            || (joystick.enabled && joystick::in_zone(touch.position, size.x, size.y))
                        { continue; }
                        let ui_owned = ui.captures(touch.position, size);
                        press_pointer(&mut layer, &mut events, source, touch.position, now, ui_owned);
                    }
                    TouchPhase::Moved => move_pointer(&mut layer, &mut events, source, touch.position, threshold),
                    TouchPhase::Ended => release_pointer(&mut layer, &mut events, source, Some(touch.position), now),
                    TouchPhase::Canceled => {
                        if layer.touch.as_ref().is_some_and(|p| p.source == source) {
                            cancel(&mut layer, &mut events);
                            layer.touch = None;
                        }
                    }
                }
            }
            WindowEvent::WindowFocused(focus) if focus.window == window_entity && !focus.focused => {
                cancel(&mut layer, &mut events);
                layer.touch = None;
            }
            WindowEvent::CursorLeft(left) if left.window == window_entity => {
                layer.cursor = None;
                // There is no pointer lock/capture in this host. Leaving cancels
                // ownership; re-entry must not integrate a jump from the old UI.
                if layer.touch.as_ref().is_some_and(|p| p.source == PointerSource::Mouse) {
                    cancel(&mut layer, &mut events);
                    layer.touch = None;
                }
            }
            _ => {}
        }
    }
    if layer.tracking == Some(GestureKind::Tap) {
        let long = layer.touch.as_mut().is_some_and(|touch| {
            if (touch.current - touch.start).length() > threshold { return false; }
            touch.touching_time = (touch.touching_time + time.delta_secs()).min(f32::MAX);
            touch.touching_time > LONG_TOUCH_THRESHOLD
        });
        if long { begin(&mut layer, &mut events, GestureKind::LongTouch); }
    }
}

fn press_pointer(
    layer: &mut GestureLayerState, events: &mut MessageWriter<GestureEvent>,
    source: PointerSource, position: Vec2, now: f32, ui_owned: bool,
) {
    if layer.touch.is_some() { return; }
    layer.touch = Some(Touch {
        start: position, current: position, delta: Vec2::ZERO,
        touching_time: 0., source, ui_owned,
    });
    let kind = if layer.last_gesture == Some(GestureKind::Tap)
        && now - layer.last_gestured_at < CONSECUTIVE_TAP_THRESHOLD {
        GestureKind::DoubleTap
    } else { GestureKind::Tap };
    begin(layer, events, kind);
}

fn move_pointer(
    layer: &mut GestureLayerState, events: &mut MessageWriter<GestureEvent>,
    source: PointerSource, position: Vec2, threshold: f32,
) {
    let Some(touch) = layer.touch.as_mut().filter(|p| p.source == source) else { return; };
    let delta = position - touch.current;
    touch.current = position;
    touch.delta = delta;
    if delta == Vec2::ZERO { return; }
    layer.drag_delta += delta.length();
    if layer.drag_delta > threshold {
        begin(layer, events, GestureKind::Drag);
        layer.drag_delta = 0.;
    }
    if layer.tracking == Some(GestureKind::Drag) {
        let touch = layer.touch.as_ref().expect("drag owns a pointer");
        events.write(GestureEvent {
            kind: GestureKind::Drag, state: GestureState::Moved,
            position: touch.current, delta: touch.delta, ui_owned: touch.ui_owned,
        });
    }
}

fn release_pointer(
    layer: &mut GestureLayerState, events: &mut MessageWriter<GestureEvent>,
    source: PointerSource, position: Option<Vec2>, now: f32,
) {
    if !layer.touch.as_ref().is_some_and(|p| p.source == source) { return; }
    let mut touch = layer.touch.take().expect("matching pointer");
    if let Some(position) = position { touch.current = position; }
    if let Some(kind) = layer.tracking.take() {
        events.write(GestureEvent {
            kind, state: GestureState::End, position: touch.current,
            delta: touch.delta, ui_owned: touch.ui_owned,
        });
        layer.last_gesture = Some(kind);
        layer.last_gestured_at = now;
        info!("[gesture] {} end @({:.0},{:.0}), UI owned={}",
            kind_name(kind), touch.current.x, touch.current.y, touch.ui_owned);
    }
    layer.drag_delta = 0.;
}

/// 源 BeginGesture：同种在跟则无事；换跟先取消旧手势（发取消事件、
/// 退跟、清零），再进新种并按种类发 Began。
fn begin(
    layer: &mut GestureLayerState,
    events: &mut MessageWriter<GestureEvent>,
    kind: GestureKind,
) {
    if layer.tracking == Some(kind) {
        return;
    }
    cancel(layer, events);
    layer.tracking = Some(kind);
    // 源 OnGestureBegan：DOUBLE_TAP（与 DUO 族）静默起手不发 Began，
    // 其余发（位 = 触点当前位）。
    if kind == GestureKind::DoubleTap {
        return;
    }
    if let Some(touch) = layer.touch.as_ref() {
        events.write(GestureEvent {
            kind,
            state: GestureState::Began,
            position: touch.current,
            delta: Vec2::ZERO,
            ui_owned: touch.ui_owned,
        });
    }
}

/// 源 CanselGesture：在跟才动手——源 OnGestureCancel 用 **End 态**发
/// 取消事件、且只对拖拽族（16/32）发，单指只剩 DRAG；随后退跟、清零
/// 累计量（不清触点——触点还在按着）。
fn cancel(layer: &mut GestureLayerState, events: &mut MessageWriter<GestureEvent>) {
    if let Some(kind) = layer.tracking {
        if kind == GestureKind::Drag {
            if let Some(touch) = layer.touch.as_ref() {
                events.write(GestureEvent {
                    kind,
                    state: GestureState::End,
                    position: touch.current,
                    delta: touch.delta,
                    ui_owned: touch.ui_owned,
                });
            }
        }
        layer.tracking = None;
        layer.drag_delta = 0.0;
    }
}


/// 日志用名。
fn kind_name(kind: GestureKind) -> &'static str {
    match kind {
        GestureKind::Tap => "TAP",
        GestureKind::DoubleTap => "DOUBLE_TAP",
        GestureKind::Drag => "DRAG",
        GestureKind::LongTouch => "LONG_TOUCH",
    }
}

#[cfg(test)]
mod playback_gesture_tests {
    use super::*;
    use bevy::ecs::system::SystemState;

    fn pointer_sequence(drag: bool, ui_owned: bool) -> Vec<GestureEvent> {
        let mut world = World::new();
        world.init_resource::<Messages<GestureEvent>>();
        let mut writer = SystemState::<MessageWriter<GestureEvent>>::new(&mut world);
        let mut layer = GestureLayerState::default();
        {
            let mut events = writer.get_mut(&mut world);
            press_pointer(&mut layer, &mut events, PointerSource::Mouse, Vec2::ZERO, 1., ui_owned);
            if drag {
                move_pointer(&mut layer, &mut events, PointerSource::Mouse, Vec2::new(40., 0.), 6.);
            }
            release_pointer(&mut layer, &mut events, PointerSource::Mouse, None, 1.1);
        }
        world.resource_mut::<Messages<GestureEvent>>().drain().collect()
    }

    #[test]
    fn camera_drag_release_cannot_be_a_dialogue_advance_tap() {
        let events = pointer_sequence(true, false);
        assert!(events.iter().any(|event| event.kind == GestureKind::Drag && event.state == GestureState::Moved));
        assert!(events.iter().any(|event| event.kind == GestureKind::Drag && event.state == GestureState::End));
        assert!(!events.iter().any(|event| event.kind.is_tap_family() && event.state == GestureState::End));
        assert!(pointer_sequence(false, false).iter().any(|event| event.kind.is_tap_family() && event.state == GestureState::End));
    }

    #[test]
    fn ui_drag_keeps_its_original_capture_through_release() {
        let events = pointer_sequence(true, true);
        assert!(events.iter().all(|event| event.ui_owned));
        assert!(!events.iter().any(|event| event.kind.is_tap_family() && event.state == GestureState::End));
    }
}
