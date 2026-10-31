//! 律：对真源可复算的式子。依赖表里没有 bevy，也不会有。
//!
//! 迁入守则见仓库 README：每族律带一份在采样输入上与真源逐值比的比对，
//! 移植时跑一次。

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
