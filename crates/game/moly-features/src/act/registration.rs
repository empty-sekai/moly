//! Registering a crate's steps: each step name, the spec or specs its fields
//! stand for, whether it writes the hand, and the timeout it has when the
//! script gives none.

use std::sync::Arc;

use bevy::prelude::*;
use serde_json::{Map, Value};

use super::{ActCtx, ActionKind, ActionRun, ActionSpec, Actions, Fields};

/// One step name, as its crate registers it.
pub struct Step<'a> {
    /// The step name (`companion.walk_to`).
    pub name: &'a str,
    /// Whether the step writes the hand: it then takes the hand lease.
    pub hand: bool,
    /// The timeout the step takes when the script gives none; a spec that
    /// works one out from its own fields still says so ([`ActionSpec::timeout`]).
    pub timeout: Option<f32>,
    /// The spec the step stands for, or the several a macro of steps stands
    /// for, each with its own hand and timeout.
    pub read: Box<dyn Fn(&mut Fields<'_>) -> Result<Vec<Arc<dyn ActionSpec>>, String> + Send + Sync + 'a>,
}

impl<'a> Step<'a> {
    /// A step that stands for one spec.
    pub fn new(
        name: &'a str,
        hand: bool,
        timeout: Option<f32>,
        read: impl Fn(&mut Fields<'_>) -> Result<Box<dyn ActionSpec>, String> + Send + Sync + 'a,
    ) -> Self {
        Self {
            name,
            hand,
            timeout,
            read: Box::new(move |fields| read(fields).map(|spec| vec![Arc::from(spec) as Arc<dyn ActionSpec>])),
        }
    }

    /// A step that stands for several specs (a macro's parts), each with its
    /// own hand and timeout: the step's own do not apply.
    pub fn many(
        name: &'a str,
        read: impl Fn(&mut Fields<'_>) -> Result<Vec<Arc<dyn ActionSpec>>, String> + Send + Sync + 'a,
    ) -> Self {
        Self { name, hand: false, timeout: None, read: Box::new(read) }
    }
}

/// The steps one crate registers for a client (`player.*`, `layout.*`).
#[derive(Default)]
pub struct Kinds(Vec<Step<'static>>);

impl Kinds {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a step; a name registered twice keeps the later one.
    pub fn step(mut self, step: Step<'static>) -> Self {
        self.0.retain(|known| known.name != step.name);
        self.0.push(step);
        self
    }

    /// Registers every step on `app`'s actions.
    pub fn install(self, app: &mut App) {
        app.world_mut().resource_mut::<Actions>().register_kinds(self);
    }
}

/// A step registered through [`Kinds`].
struct Stepped(&'static Step<'static>);

impl ActionKind for Stepped {
    fn name(&self) -> &'static str {
        self.0.name
    }

    fn hand(&self) -> bool {
        self.0.hand
    }

    fn parse(&self, fields: &Map<String, Value>, at: &str) -> Result<Vec<Arc<dyn ActionSpec>>, String> {
        let mut reader = Fields::new(fields, at);
        let specs = (self.0.read)(&mut reader)?;
        reader.finish()?;
        Ok(specs.into_iter().map(|spec| Arc::new(StepSpec { step: self.0, spec }) as Arc<dyn ActionSpec>).collect())
    }
}

/// A step's spec, with the step's own hand and timeout.
struct StepSpec {
    step: &'static Step<'static>,
    spec: Arc<dyn ActionSpec>,
}

impl ActionSpec for StepSpec {
    fn name(&self) -> &str {
        self.spec.name()
    }

    fn hand(&self) -> bool {
        self.step.hand || self.spec.hand()
    }

    fn default_timeout(&self) -> Option<f32> {
        self.step.timeout.or_else(|| self.spec.timeout())
    }

    fn start(&self, world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        self.spec.start(world, ctx)
    }
}

impl std::fmt::Debug for StepSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("StepSpec").field(&self.spec).finish()
    }
}

/// Registers the steps `kinds` gave on `actions`.
pub(crate) fn register_kinds(actions: &mut Actions, mut kinds: Kinds) {
    for step in std::mem::take(&mut kinds.0) {
        let step: &'static Step<'static> = Box::leak(Box::new(step));
        actions.register(Stepped(step));
    }
}
