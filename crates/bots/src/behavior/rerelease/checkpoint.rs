//! Rerelease behavior checkpoint from `src/bots/behavior/rerelease/checkpoint.ts`.
//!
//! Behavior state is explicit memory plus RNG state, keyed by source
//! name. `rerelease_behavior_checkpoint` captures it; `restore`
//! validates skill identity, rejects foreign skills and versions, and
//! never shares live memory with the image.

use std::collections::HashMap;

use crate::behavior::rerelease::brain::{BotBrain, BotBrainCheckpoint};
use crate::error::BotsError;

/// One bot's behavior image.
#[derive(Debug, Clone)]
pub struct RereleaseBehaviorBotCheckpoint {
    /// Brain image.
    pub brain: BotBrainCheckpoint,
    /// RNG state word.
    pub rng: i32,
}

/// Behavior checkpoint: images by source name.
#[derive(Debug, Clone, Default)]
pub struct RereleaseBehaviorCheckpoint {
    /// Images by bot name.
    pub bots: HashMap<String, RereleaseBehaviorBotCheckpoint>,
}

impl RereleaseBehaviorCheckpoint {
    /// Empty checkpoint.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Bot count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bots.len()
    }

    /// Whether empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bots.is_empty()
    }
}

/// Capture behavior images for named brains.
#[must_use]
pub fn rerelease_behavior_checkpoint(brains: &HashMap<String, &BotBrain>) -> RereleaseBehaviorCheckpoint {
    let mut checkpoint = RereleaseBehaviorCheckpoint::new();
    for (name, brain) in brains {
        checkpoint.bots.insert(
            name.clone(),
            RereleaseBehaviorBotCheckpoint {
                brain: brain.checkpoint(),
                rng: brain.rng_state(),
            },
        );
    }
    checkpoint
}

/// Restore one brain from its image.
pub fn restore_rerelease_bot(brain: &mut BotBrain, image: &RereleaseBehaviorBotCheckpoint) -> Result<(), BotsError> {
    brain.restore(&image.brain)?;
    brain.restore_rng(image.rng)?;
    Ok(())
}

/// Restore named brains from a checkpoint. Unknown names fail; missing
/// names are left untouched.
pub fn restore_rerelease_behavior(
    brains: &mut HashMap<String, &mut BotBrain>,
    checkpoint: &RereleaseBehaviorCheckpoint,
) -> Result<(), BotsError> {
    for (name, image) in &checkpoint.bots {
        let Some(brain) = brains.get_mut(name) else {
            return Err(BotsError::BotCheckpoint(format!(
                "bot behavior checkpoint names an unknown bot \"{name}\""
            )));
        };
        restore_rerelease_bot(brain, image)?;
    }
    Ok(())
}
