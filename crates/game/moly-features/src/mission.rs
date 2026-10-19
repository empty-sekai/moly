//! The mission screen's data model: the pure rules over the mission master
//! tables and a plain user state.
//!
//! The model decides what the screen shows and what a receive asks for; it
//! holds no Bevy resource, loads nothing and draws nothing. The tables are
//! [`moly_masters::mission::MissionMasters`] (sheets, missions, missions'
//! reward rows and the missions' rank-experience rows); the user data is the
//! sheet the user is on and the progress rows the server keeps.

pub mod model;
