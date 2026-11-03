//! The picture over the field: captions and the screen's colour.
//!
//! The player owns one layer of bevy UI nodes, drawn over the whole window on
//! the HUD camera's UI pass, above the field's own UI: a full-window block of
//! the screen colour, and one place per caption style. The style chooses where
//! its place sits: the subtitle low over the lower middle, the location in
//! the lower left corner, the title in the upper third, and a full-screen
//! title in the middle. Each place takes its caption's text, its opacity and
//! a plate whose alpha follows the opacity ([`place`], [`font_size`] and
//! [`ink`] are the style's other choices). The nodes are made on the first
//! frame with something to show, hidden while nothing shows, and removed when
//! the document goes.

use bevy::prelude::*;
use bevy::text::FontSize;
use bevy::ui::{FocusPolicy, UiTargetCamera, Val};
use moly_features::balloon::HudCamera;
use moly_features::ui_font::UiFontAsset;
use moly_sequence::doc::CaptionStyle;
use moly_sequence::CaptionFrame;

use crate::Player;

/// The places, one per caption style, in the order of their slots.
const STYLES: [CaptionStyle; 4] = [CaptionStyle::Subtitle, CaptionStyle::Location, CaptionStyle::Title, CaptionStyle::Fullscreen];

/// The layer's root, above the field's UI.
const LAYER_Z: i32 = 900;

/// The alpha of the plate behind a caption at full opacity.
const PLATE_ALPHA: f32 = 0.4;

fn slot(style: CaptionStyle) -> usize {
    match style {
        CaptionStyle::Subtitle => 0,
        CaptionStyle::Location => 1,
        CaptionStyle::Title => 2,
        CaptionStyle::Fullscreen => 3,
    }
}

/// The ink a caption is written in.
fn ink(style: CaptionStyle) -> Srgba {
    match style {
        CaptionStyle::Subtitle => Color::WHITE.to_srgba(),
        CaptionStyle::Location => Color::srgb(0.98, 0.94, 0.82).to_srgba(),
        CaptionStyle::Title | CaptionStyle::Fullscreen => Color::srgb(1.0, 0.99, 0.95).to_srgba(),
    }
}

fn font_size(style: CaptionStyle) -> f32 {
    match style {
        CaptionStyle::Subtitle => 26.0,
        CaptionStyle::Location => 22.0,
        CaptionStyle::Title => 40.0,
        CaptionStyle::Fullscreen => 52.0,
    }
}

/// A place's node: where over the window it sits and how its text lines up.
fn place(style: CaptionStyle) -> Node {
    let band = |bottom: Val, top: Val, width: Val, justify: JustifyContent| Node {
        position_type: PositionType::Absolute,
        left: Val::Px(0.0),
        bottom,
        top,
        width,
        padding: UiRect::all(Val::Px(12.0)),
        justify_content: justify,
        align_items: AlignItems::Center,
        ..default()
    };
    match style {
        CaptionStyle::Subtitle => band(Val::Px(64.0), Val::Auto, Val::Percent(100.0), JustifyContent::Center),
        CaptionStyle::Title => band(Val::Auto, Val::Px(96.0), Val::Percent(100.0), JustifyContent::Center),
        CaptionStyle::Location => Node {
            position_type: PositionType::Absolute,
            left: Val::Px(40.0),
            bottom: Val::Px(44.0),
            padding: UiRect::all(Val::Px(12.0)),
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::FlexStart,
            ..default()
        },
        CaptionStyle::Fullscreen => Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
    }
}

/// One caption place.
#[derive(Component)]
pub struct CaptionPlace;

/// The screen colour block.
#[derive(Component)]
pub struct ScreenBlock;

/// The nodes of the layer.
struct Layer {
    root: Entity,
    block: Entity,
    places: [Entity; 4],
    texts: [Entity; 4],
    /// What each place shows: (text, opacity), and the block's colour.
    shown: [Option<(String, f32)>; 4],
    colour: Option<[u32; 4]>,
}

/// The overlay: the frame's texts and screen colour, and the nodes drawn.
#[derive(Default)]
pub struct Overlay {
    texts: [Option<(String, f32)>; 4],
    screen: Option<[f32; 4]>,
    layer: Option<Layer>,
}

impl Overlay {
    /// Takes the frame's captions (a place takes the topmost caption laid on
    /// it) and screen colour.
    pub fn lay(&mut self, captions: &[CaptionFrame<'_>], screen: Option<[f32; 4]>) {
        self.texts = Default::default();
        for caption in captions {
            self.texts[slot(caption.style)] = Some((caption.text.to_owned(), caption.opacity.clamp(0.0, 1.0)));
        }
        self.screen = screen;
    }

    /// Nothing shows.
    fn empty(&self) -> bool {
        self.screen.is_none() && self.texts.iter().all(Option::is_none)
    }

    /// Takes the layer away and forgets the frame.
    pub fn hide(&mut self, world: &mut World) {
        self.texts = Default::default();
        self.screen = None;
        if let Some(layer) = self.layer.take() {
            if world.get_entity(layer.root).is_ok() {
                let _ = world.despawn(layer.root);
            }
        }
    }

    /// Draws the frame on the layer, making it on the first frame with
    /// something to show.
    fn draw(&mut self, world: &mut World) {
        if let Some(layer) = self.layer.as_ref() {
            if world.get_entity(layer.root).is_err() {
                self.layer = None;
            }
        }
        if self.layer.is_none() {
            if self.empty() {
                return;
            }
            let Some(camera) = world.query_filtered::<Entity, With<HudCamera>>().iter(world).next() else {
                return;
            };
            self.layer = Some(build(world, camera));
        }
        let empty = self.empty();
        let Some(layer) = self.layer.as_mut() else {
            return;
        };
        world.entity_mut(layer.root).insert(if empty { Visibility::Hidden } else { Visibility::Inherited });
        for (index, style) in STYLES.into_iter().enumerate() {
            let shown = self.texts[index].clone();
            if shown != layer.shown[index] {
                let (text, alpha) = shown.clone().unwrap_or_else(|| (String::new(), 0.0));
                let srgba = ink(style);
                world.entity_mut(layer.texts[index]).insert((Text::new(text), TextColor(Color::Srgba(srgba.with_alpha(alpha)))));
                world.entity_mut(layer.places[index]).insert((
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, PLATE_ALPHA * alpha)),
                    if shown.is_some() { Visibility::Inherited } else { Visibility::Hidden },
                ));
                layer.shown[index] = shown;
            }
        }
        let colour = self.screen.map(|colour| colour.map(f32::to_bits));
        if colour != layer.colour {
            match self.screen {
                Some([r, g, b, a]) => {
                    let clamp = |value: f32| value.clamp(0.0, 1.0);
                    world.entity_mut(layer.block).insert((BackgroundColor(Color::srgba(clamp(r), clamp(g), clamp(b), clamp(a))), Visibility::Inherited));
                }
                None => {
                    world.entity_mut(layer.block).insert(Visibility::Hidden);
                }
            }
            layer.colour = colour;
        }
    }
}

/// Makes the layer's nodes on the HUD camera's UI pass.
fn build(world: &mut World, camera: Entity) -> Layer {
    let font = world.get_resource::<UiFontAsset>().map(|font| font.0.clone());
    let root = world
        .spawn((
            Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
            Visibility::Inherited,
            UiTargetCamera(camera),
            GlobalZIndex(LAYER_Z),
        ))
        .id();
    let block = world
        .spawn((
            ScreenBlock,
            Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
            BackgroundColor(Color::NONE),
            Visibility::Hidden,
        ))
        .id();
    world.entity_mut(root).add_child(block);
    let mut places = [Entity::PLACEHOLDER; 4];
    let mut texts = [Entity::PLACEHOLDER; 4];
    for (index, style) in STYLES.into_iter().enumerate() {
        let mut text_font = TextFont { font_size: FontSize::Px(font_size(style)), ..default() };
        if let Some(font) = &font {
            text_font.font = font.clone().into();
        }
        let text = world
            .spawn((
                Text::new(""),
                TextLayout::no_wrap(),
                TextColor(Color::NONE),
                text_font,
                Node { max_width: Val::Px(1200.0), ..default() },
            ))
            .id();
        let place = world.spawn((CaptionPlace, place(style), BackgroundColor(Color::NONE), Visibility::Hidden, FocusPolicy::Pass)).id();
        let mut caption = world.entity_mut(place);
        caption.add_child(text);
        caption.insert(ChildOf(root));
        places[index] = place;
        texts[index] = text;
    }
    Layer { root, block, places, texts, shown: Default::default(), colour: None }
}

/// Update (after the player's frame): draws the captions and screen colour of
/// the frame the player laid.
pub fn overlay(world: &mut World) {
    let Some(mut player) = world.remove_resource::<Player>() else {
        return;
    };
    if player.doc.is_some() {
        player.overlay.draw(world);
    }
    world.insert_resource(player);
}
