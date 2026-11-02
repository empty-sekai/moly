//! A spec a reader of this crate starts straight, with its step's own hand
//! and timeout, for the steps that stand for several actions
//! (`player.craft`'s walk to the workbench and its craft).

use std::sync::Arc;

use bevy::prelude::*;
use moly_features::act::{ActionRun, ActionSpec, ActCtx};

/// The spec one reader built, with its step's hand and timeout.
#[derive(Debug)]
pub struct Gated {
    hand: bool,
    timeout: Option<f32>,
    spec: Box<dyn ActionSpec>,
}

impl Gated {
    pub fn new(spec: impl ActionSpec, hand: bool, timeout: Option<f32>) -> Self {
        Self { hand, timeout, spec: Box::new(spec) }
    }

    pub fn arc(self) -> Arc<dyn ActionSpec> {
        Arc::new(self)
    }
}

impl ActionSpec for Gated {
    fn name(&self) -> &str {
        self.spec.name()
    }

    fn hand(&self) -> bool {
        self.hand || self.spec.hand()
    }

    fn default_timeout(&self) -> Option<f32> {
        self.timeout.or_else(|| self.spec.timeout())
    }

    fn start(&self, world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        self.spec.start(world, ctx)
    }
}
