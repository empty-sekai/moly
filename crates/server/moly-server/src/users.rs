//! The users the server keeps a document for.
//!
//! A user is seated when its first session says hello: from the document the
//! store holds, else from the product default. Its `Join` call seats it in
//! the masters and answers with every section. The clock, the seated birthday
//! parties and the store writes are systems over every user.

use std::collections::HashMap;

use bevy::prelude::*;
use moly_protocol::{PushKind, ServerMessage, UserId};

use crate::defaults::Defaults;
use crate::hub::{SessionOpened, Sessions};
use crate::masters::{Masters, Parties, Tables};
use crate::model::delivery::PartyMaster;
use moly_masters::load::MasterTables;
use crate::model::ResponseKind;
use crate::model::document;
use crate::model::{Origin, ServerModel};
use crate::store::Store;

/// Seconds between the running clock's refresh checks.
const TICK_SECONDS: f32 = 1.0;

/// One user's model, or why it could not be seated.
pub(crate) struct UserEntry {
    pub(crate) model: Option<ServerModel>,
    /// The stored document was refused; every call answers with it.
    pub(crate) load_error: Option<String>,
}

/// The users the server knows.
#[derive(Resource, Default)]
pub struct Users(pub(crate) HashMap<UserId, UserEntry>);

pub(crate) fn install(app: &mut App) {
    app.init_resource::<Users>().add_systems(
        PreUpdate,
        (create_users.after(crate::hub::pump), seat_users).chain(),
    );
}

/// A session said hello: the user is kept, its model waits for the default
/// document and the store.
fn create_users(mut opened: MessageReader<SessionOpened>, mut users: ResMut<Users>) {
    for event in opened.read() {
        users
            .0
            .entry(event.user.clone())
            .or_insert_with(|| UserEntry {
                model: None,
                load_error: None,
            });
    }
}

/// Seats a user's model: the store's document when it has one, else the
/// product default.
fn seat_users(
    mut users: ResMut<Users>,
    defaults: Option<Res<Defaults>>,
    store: Res<Store>,
) {
    let Some(defaults) = defaults.as_deref() else {
        return;
    };
    for (user, entry) in &mut users.0 {
        if entry.model.is_some() || entry.load_error.is_some() {
            continue;
        }
        match store.0.load(user) {
            Some(text) => match document::parse_v2(&text) {
                Ok((doc, pending)) => {
                    let mut model = ServerModel::new(doc, Origin::Seed, pending);
                    model.avatar_instrument = defaults.avatar_instrument.clone();
                    info!("[server] user {user}: document from the store");
                    entry.model = Some(model);
                }
                Err(reason) => {
                    error!("[server] user {user}: the stored document is refused: {reason}");
                    entry.load_error = Some(format!(
                        "the stored document of user {user} is refused: {reason}"
                    ));
                }
            },
            None => {
                let mut model = ServerModel::new(
                    defaults.doc.clone(),
                    defaults.origin,
                    Vec::new(),
                );
                model.avatar_instrument = defaults.avatar_instrument.clone();
                info!("[server] user {user}: no stored document; the product default is seated");
                entry.model = Some(model);
            }
        }
    }
}

/// The running clock's refresh check, every second.
pub(crate) fn tick(
    mut users: ResMut<Users>,
    masters: Res<Masters>,
    shared: Option<Res<MasterTables>>,
    time: Res<Time<Real>>,
    mut elapsed: Local<f32>,
) {
    let tables = Tables::new(&masters, shared.as_deref());
    *elapsed += time.delta_secs();
    if *elapsed < TICK_SECONDS {
        return;
    }
    *elapsed = 0.0;
    if !masters.ready {
        return;
    }
    for entry in users.0.values_mut() {
        if let Some(model) = &mut entry.model {
            if model.joined {
                model.tick(&tables);
            }
        }
    }
}

/// The birthday parties in session at each user's server clock: the new row
/// and delivery item stock policies, answered as a response when they seat a
/// row.
pub(crate) fn seat_parties(
    mut users: ResMut<Users>,
    masters: Res<Masters>,
    shared: Option<Res<MasterTables>>,
    parties: Res<Parties>,
) {
    let tables = Tables::new(&masters, shared.as_deref());
    if !masters.ready {
        return;
    }
    for (user, entry) in &mut users.0 {
        let Some(model) = &mut entry.model else {
            continue;
        };
        model.parties_missing = parties.missing().map(str::to_owned);
        if !model.joined {
            continue;
        }
        let now = model.now_ms();
        let mut in_session = Vec::new();
        let mut named = Vec::new();
        for row in parties.in_session(now) {
            match (
                i32::try_from(row.id),
                i32::try_from(row.delivery_item_material_id),
                i32::try_from(row.delivery_reward_material_id),
            ) {
                (Ok(id), Ok(item), Ok(reward)) => in_session.push(PartyMaster {
                    birthday_party_id: id,
                    delivery_item_material_id: item,
                    delivery_reward_mysekai_material_id: reward,
                }),
                _ => model.push_error(format!(
                    "birthday party {} has an id that does not fit a 32-bit integer",
                    row.label
                )),
            }
            named.push(row.label);
        }
        if in_session == model.party_masters {
            continue;
        }
        let changed = model.seat_parties(&in_session);
        info!(
            "[server] user {user}: birthday parties in session at {now}: {named:?}"
        );
        if !changed.is_empty() {
            model.commit();
            model.respond(
                ResponseKind::BirthdayPartySeat,
                false,
                &changed,
                &tables,
            );
        }
    }
}

/// Sends the responses the server made on its own — a new refresh window, the
/// seated parties, a synchronised edit — to every session of their user.
pub(crate) fn flush_pushes(mut users: ResMut<Users>, sessions: Res<Sessions>) {
    for (user, entry) in &mut users.0 {
        let Some(model) = &mut entry.model else {
            continue;
        };
        let mut kept = Vec::new();
        for response in std::mem::take(&mut model.responses) {
            match push_kind(response.kind) {
                Some(kind) => sessions.broadcast(
                    user,
                    &ServerMessage::Push {
                        kind,
                        sections: Box::new(response.sections),
                    },
                ),
                None => kept.push(response),
            }
        }
        model.responses = kept;
    }
}

fn push_kind(kind: ResponseKind) -> Option<PushKind> {
    match kind {
        ResponseKind::Refresh => Some(PushKind::Refresh),
        ResponseKind::Sync => Some(PushKind::Sync),
        ResponseKind::BirthdayPartySeat => Some(PushKind::BirthdayPartySeat),
        _ => None,
    }
}

/// Writes every user whose document changed since the last write.
pub(crate) fn save_changed(mut users: ResMut<Users>, store: Res<Store>) {
    for (user, entry) in &mut users.0 {
        let Some(model) = &mut entry.model else {
            continue;
        };
        if model.changed {
            store.0.save(user, &model.text());
            model.changed = false;
        }
    }
}
