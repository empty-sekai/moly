//! The source stencil as the main pass sees it.
//!
//! The product's main pass has no stencil. A stencil replay (in the world
//! crate) replays the stencil-relevant draws in queue order into a target of
//! its own and writes what the main-pass programs need into one texture, the
//! clip mask, at the size of the first active 3D camera's target:
//!
//! - red, green, blue: 1 where a program's stencil test passes at its queue
//!   (red the room wall, green the door outside, blue the ground);
//! - alpha: the stencil value itself after the last replayed draw, divided
//!   by 255, for the character eyebrow pass, which tests Equal against the
//!   character's own reference and runs after the transparents.
//!
//! A program reads its texel at its fragment's pixel. A pixel outside the
//! mask reads as a passing test (the colour channels) and as no stencil
//! value (alpha), which keeps a view of another size from reading a pixel
//! of the first camera's view.

use bevy::prelude::*;
use bevy::render::extract_resource::ExtractResource;
use bevy::render::render_resource::{Extent3d, TextureFormat};
use serde_json::Value;

/// The clip mask's format: one byte per channel holds a test result exactly
/// and a stencil value exactly (value / 255).
pub const CLIP_MASK_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

/// The clip mask's texture and the size the main world gave it.
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(bevy::render::RenderApp)]
pub struct StencilClipMask {
    pub image: Handle<Image>,
    size: UVec2,
}

impl StencilClipMask {
    pub fn new(image: Handle<Image>, size: UVec2) -> Self {
        Self { image, size }
    }

    pub fn size(&self) -> UVec2 {
        self.size
    }

    pub fn set_size(&mut self, size: UVec2) {
        self.size = size;
    }

    /// Whether a prepared texture has the size the main world gave the mask.
    pub fn fits(&self, size: Extent3d) -> bool {
        size.width == self.size.x && size.height == self.size.y
    }
}

/// The source shader attribute of a draw whose program the product does not
/// run (a room door's parts, a fixture's colourless stencil mesh), read from
/// its material by [`source_attribute`].
#[derive(Component, Clone, Copy, Debug)]
pub struct SourceStencilAttribute(pub u8);

/// The object program's usage switch, its cells with a fixed attribute.
/// Usages 0 and 14 go to the ground lookup, 4 and 13 to the basic-fixture
/// lookup and 8 to the field-object lookup, each of which reads further
/// properties; those are not decided by the usage.
pub fn object_usage_attribute(usage: u32) -> Option<u8> {
    Some(match usage {
        1 => 2,
        2 => 3,
        3 => 4,
        5 => 7,
        6 => 8,
        7 => 9,
        9 => 12,
        10 => 14,
        11 => 18,
        12 => 17,
        15 => 22,
        _ => return None,
    })
}

/// The shader attribute of a material by its program name and float
/// properties: the object program by its usage, the basic-fixture program by
/// its usage and blend mode (a property the material lacks reads 0, as the
/// source lookup reads it). Other programs have none here.
pub fn source_attribute(shader: &str, floats: &Value) -> Option<u8> {
    let whole = |key: &str| -> Option<f64> {
        floats[key]
            .as_f64()
            .filter(|v| v.fract() == 0.0 && *v >= 0.0)
    };
    match shader {
        "Mysekai/Object" => object_usage_attribute(whole("_ObjectShaderUsage")? as u32),
        "Mysekai/Fixture/Basic" => {
            let usage = whole("_FixtureShaderUsage").unwrap_or(0.0) as i32;
            let blend = whole("_FixtureObjectBlendMode").unwrap_or(0.0) as i32;
            moly_law::fixture::family::basic_fixture_shader_attribute(usage, blend)
                .ok()
                .and_then(|a| u8::try_from(a).ok())
        }
        _ => None,
    }
}
