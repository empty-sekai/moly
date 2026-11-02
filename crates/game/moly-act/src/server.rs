//! `server.edit`: one edit of the server document.
//!
//! The edit is the command object `moly.server.edit` takes (a field path and
//! a value, or an action with its fields); the server model checks it when
//! the edit is sent, and the step ends on its answer, or fails with its
//! refusal by name.

use bevy::prelude::*;
use moly_features::act::{ActionRun, ActionSpec, ActCtx, ActPoll, Fields, Kinds, Step};
use moly_features::userdata::{admin_call, Answered, AnsweredLog};
use moly_link::ServerLink;
use moly_protocol::AdminCall;
use serde_json::{Map, Value};

pub fn register(app: &mut App) {
    Kinds::new()
        .step(Step::new("server.edit", false, None, |fields| Ok(Box::new(ServerEdit(read_edit(fields)?)))))
        .install(app);
}

/// One `server.edit` step.
#[derive(Debug)]
struct ServerEdit(Map<String, Value>);

fn read_edit(fields: &mut Fields<'_>) -> Result<Map<String, Value>, String> {
    let at = fields.at().to_owned();
    // The step's other fields are the edit; the server model checks them
    // when the step runs and names what it refuses.
    let mut command = fields.rest();
    if !command.contains_key("path") && !command.contains_key("action") {
        return Err(format!("{at}: a server edit has a path and a value, or an action"));
    }
    command.insert("type".into(), "server.edit".into());
    Ok(command)
}

impl ActionSpec for ServerEdit {
    fn name(&self) -> &str {
        "server.edit"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let command_json = Value::Object(self.0.clone()).to_string();
        let seq = admin_call(world, AdminCall::Edit { command_json }).ok_or("the game has no server link")?;
        Ok(Box::new(ServerEditRun { seq }))
    }
}

/// An edit until its answer, read from the last answers by its sequence
/// number: no call was sent between this one and its answer, so the newest
/// answer from that number on is the edit's own.
struct ServerEditRun {
    seq: u32,
}

impl ActionRun for ServerEditRun {
    fn poll(&mut self, world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        let seq = self.seq;
        let answered: Option<Answered> = world.get_resource::<AnsweredLog>().and_then(|log| log.get(seq)).cloned();
        if let Some(answer) = answered {
            return match &answer.outcome {
                Ok(_) => ActPoll::Done(describe_answer(&answer)),
                Err(refusal) => ActPoll::Failed(format!("{}: {}", describe_answer(&answer), refusal.reason)),
            };
        }
        let waiting = world.get_resource::<ServerLink>().is_some_and(|link| link.waiting().any(|(waiting, _)| *waiting == seq));
        if waiting {
            ActPoll::Running(format!("waiting for the server's answer (seq {seq})"))
        } else {
            ActPoll::Failed(format!("the server edit (seq {seq}) has no answer"))
        }
    }

    fn stop(&mut self, _world: &mut World, _ctx: &mut ActCtx) {}
}

/// A server answer as a step's note reports it.
pub fn describe_answer(answer: &Answered) -> String {
    let call = answer.call.name();
    match &answer.outcome {
        Ok(_) => format!("{{seq: {}, call: {call}, ok}}", answer.seq),
        Err(refusal) => format!("{{seq: {}, call: {call}, refusal: {}}}", answer.seq, refusal.code),
    }
}
