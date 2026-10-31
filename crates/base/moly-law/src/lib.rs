//! Rules: formulas recomputable against the game. There is no bevy in
//! the dependency table, and there never will be.
//!
//! Migration guidelines are in the repository README: each rule family
//! carries a comparison that checks it value by value against the game
//! on sampled inputs, run once when the family is added.

#![allow(clippy::excessive_precision, reason = "floating-point literals keep the full decimal text of the reference values so that each one can be checked value by value")]
#![allow(clippy::neg_cmp_op_on_partial_ord, reason = "the negated form gives a different result from its positive counterpart for NaN inputs, so the written comparison decides the NaN path")]

pub mod alone_action;
pub mod bulk_harvest;
pub mod carve;
pub mod cloth;
pub mod fixture;
pub mod frame_time;
pub mod facial;
pub mod material;
pub mod npc_hfsm;
pub mod objective;
pub mod particle;
pub mod path;
pub mod powf;
pub mod shading;
pub mod talk;
pub mod text;
pub mod tweet;
pub mod ui;
pub mod weather;
pub mod action_button;
pub mod animator;
pub mod blink;
pub mod delivery;
