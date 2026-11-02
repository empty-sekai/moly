//! Output leases: who may drive a shared output of the game, the hand that
//! writes input, the camera, a character's body, or anything else a program
//! names with a lease of its own.
//!
//! A lease is taken by a holder, which is a name and a rank: a person, the
//! director's script, a sequence's player or a plugin. A holder of a rank at
//! least the sitting owner's takes the lease at once, and the owner is
//! revoked with the reason the next poll of its work ends with. A holder of a
//! lower rank is refused by name. The leases are held here, above every
//! program that takes one, so a program decides what to do with its revoked
//! leases and a person's input can take the hand from any of them.

use std::collections::BTreeMap;
use std::fmt;

use bevy::prelude::*;

/// Who holds a lease: the name it reports and its rank.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holder {
    pub id: String,
    pub rank: Rank,
}

impl Holder {
    pub fn new(id: impl Into<String>, rank: Rank) -> Self {
        Self { id: id.into(), rank }
    }
}

/// How far a holder stands above another: a holder takes a lease from one of
/// the same or a lower rank.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rank {
    Plugin = 1,
    Sequence = 2,
    Director = 3,
    Human = 4,
}

impl fmt::Display for Rank {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Plugin => "plugin",
            Self::Sequence => "sequence",
            Self::Director => "director",
            Self::Human => "human",
        })
    }
}

/// An output one holder at a time may write.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Channel {
    /// The virtual hand: the joystick finger, the tap and drag fingers and
    /// the wheel.
    Hand,
    /// The field camera.
    Camera,
    /// A character's body.
    Character(u32),
    /// An output a program names for itself.
    Custom(String),
}

impl Channel {
    /// The channel as a note names it.
    pub fn name(&self) -> String {
        match self {
            Self::Hand => "hand".to_owned(),
            Self::Camera => "camera".to_owned(),
            Self::Character(unit) => format!("character {unit}"),
            Self::Custom(name) => name.clone(),
        }
    }
}

/// Why a lease was not taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub channel: Channel,
    /// The holder that keeps the lease.
    pub holder: Holder,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} holds {}", self.holder.id, self.channel.name())
    }
}

/// The leases taken, by channel, and the leases taken from a holder since it
/// last looked, with the reason its work ends.
#[derive(Resource, Default)]
pub struct Leases {
    held: BTreeMap<Channel, Holder>,
    taken: BTreeMap<String, Vec<(Channel, String)>>,
}

impl Leases {
    /// Takes `channel` for `holder`: refused while a holder of the same or a
    /// higher rank keeps it. A keeper of a lower rank is revoked.
    pub fn take(&mut self, channel: Channel, holder: &Holder) -> Result<(), Refusal> {
        if let Some(sitting) = self.held.get(&channel) {
            if sitting.id == holder.id {
                return Ok(());
            }
            if sitting.rank >= holder.rank {
                return Err(Refusal { channel, holder: sitting.clone() });
            }
        }
        if let Some(sitting) = self.held.insert(channel.clone(), holder.clone()) {
            let reason = format!("{} took {}", holder.id, channel.name());
            self.taken.entry(sitting.id).or_default().push((channel, reason));
        }
        Ok(())
    }

    /// Lets `channel` go when `holder_id` is its keeper.
    pub fn release(&mut self, channel: &Channel, holder_id: &str) {
        if self.held.get(channel).is_some_and(|sitting| sitting.id == holder_id) {
            self.held.remove(channel);
        }
    }

    /// Lets every lease `holder_id` keeps go.
    pub fn release_all(&mut self, holder_id: &str) {
        self.held.retain(|_, sitting| sitting.id != holder_id);
        self.taken.remove(holder_id);
    }

    /// Lets every lease go whose holder's name is `prefix` or starts with
    /// `prefix` and a colon: a program's own leases, whichever of its steps
    /// took them.
    pub fn release_prefix(&mut self, prefix: &str) {
        self.held.retain(|_, sitting| {
            !(sitting.id == prefix || sitting.id.strip_prefix(prefix).is_some_and(|rest| rest.starts_with(':')))
        });
        self.taken.retain(|holder, _| !(holder == prefix || holder.strip_prefix(prefix).is_some_and(|rest| rest.starts_with(':'))));
    }

    /// The holder of `channel`.
    pub fn holder(&self, channel: &Channel) -> Option<&Holder> {
        self.held.get(channel)
    }

    /// The leases `holder_id` keeps, with the holder that took each.
    pub fn of(&self, holder_id: &str) -> Vec<(Channel, Holder)> {
        self.held.iter().filter(|(_, sitting)| sitting.id == holder_id).map(|(channel, sitting)| (channel.clone(), sitting.clone())).collect()
    }

    /// The leases `holder_id` has kept and holds no more, each with the
    /// reason its holder's work ends with; they are forgotten.
    pub fn revoked(&mut self, holder_id: &str) -> Vec<(Channel, String)> {
        match self.taken.remove(holder_id) {
            Some(revoked) => revoked.into_iter().filter(|(channel, _)| self.held.get(channel).is_none_or(|sitting| sitting.id != holder_id)).collect(),
            None => Vec::new(),
        }
    }

    /// The holder that took a lease from each holder since it last looked.
    pub fn revoked_holders(&self) -> std::collections::BTreeSet<String> {
        self.taken.keys().cloned().collect()
    }
}
