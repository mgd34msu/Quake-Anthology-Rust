//! game.h memory projections; all physics executes in qa_movement.
use crate::{
    memory::{ModuleMemory, Width},
    services::CallError,
};
use qa_core::primitives::{ModuleId, PlayerState, ResourceNamespace, UserCmd};
use qa_network::projection::{
    GRAVITY_WORD, MOVEMENT_WORDS, PM_FLAGS_WORD, PlayerContext, PlayerProjection,
};

use Width::*;
/// Original x86-64 game.h offsets, checked with sizeof/offsetof on qsrc.
pub(crate) struct Outputs {
    pub jump_sound: usize,
    pub step_clip: usize,
    pub impact_delta: usize,
}
pub(crate) struct Layout {
    state: [(usize, Width); MOVEMENT_WORDS],
    pub bytes: usize,
    pub command: usize,
    initial_snap: (usize, Width),
    pub outputs: Option<Outputs>,
    pub touches: usize,
    pub touch_count: usize,
    pub view: usize,
    pub height: Option<usize>,
    pub mins: usize,
    pub maxs: usize,
    pub ground: usize,
    pub water: usize,
    pub level: usize,
    pub player: Option<usize>,
    pub wide: bool,
}
pub(crate) const CLASSIC: Layout = Layout {
    state: [
        (0, Word),
        (4, Short),
        (6, Short),
        (8, Short),
        (10, Short),
        (12, Short),
        (14, Short),
        (17, Byte),
        (16, Byte),
        (18, Short),
        (20, Short),
        (22, Short),
        (24, Short),
    ],
    bytes: 384,
    command: 28,
    initial_snap: (44, Word),
    outputs: None,
    touches: 56,
    touch_count: 48,
    view: 312,
    height: Some(324),
    mins: 328,
    maxs: 340,
    ground: 352,
    water: 360,
    level: 364,
    player: None,
    wide: false,
};
pub(crate) const RERELEASE: Layout = Layout {
    state: [
        (0, Word),
        (4, Word),
        (8, Word),
        (12, Word),
        (16, Word),
        (20, Word),
        (24, Word),
        (30, Short),
        (28, Short),
        (32, Short),
        (36, Word),
        (40, Word),
        (44, Word),
    ],
    bytes: 3320,
    command: 52,
    initial_snap: (80, Byte),
    outputs: Some(Outputs {
        jump_sound: 3309,
        step_clip: 3310,
        impact_delta: 3312,
    }),
    touches: 96,
    touch_count: 88,
    view: 3168,
    height: None,
    mins: 3180,
    maxs: 3192,
    ground: 3208,
    water: 3236,
    level: 3240,
    player: Some(3248),
    wide: true,
};
pub struct Pmove {
    pub(crate) layout: &'static Layout,
    projection: PlayerProjection,
    client: &'static crate::q2_abi::ClientLayout,
    words: Box<[u32]>,
}
impl Pmove {
    pub fn options(
        &self,
        memory: &ModuleMemory<'_>,
        address: u64,
    ) -> Result<qa_movement::MoveOptions, CallError> {
        Ok(qa_movement::MoveOptions {
            initial_snap: memory.read_field(
                address + self.layout.initial_snap.0 as u64,
                self.layout.initial_snap.1,
            )? != 0,
            ..Default::default()
        })
    }
    pub(crate) fn load(
        layout: &'static Layout,
        client: &'static crate::q2_abi::ClientLayout,
    ) -> Self {
        let projection = PlayerProjection::load_movement(client.stats.protocol);
        let words = vec![0; projection.words()].into_boxed_slice();
        Self {
            layout,
            projection,
            client,
            words,
        }
    }
    pub fn read_state(
        &mut self,
        memory: &ModuleMemory<'_>,
        address: u64,
        module: ModuleId,
        player: &mut PlayerState,
    ) -> Result<(), CallError> {
        for (word, &(offset, width)) in self.words.iter_mut().zip(&self.layout.state) {
            *word = memory.read_field(address + offset as u64, width)?;
        }
        // Gravity is a signed short even though positions/angles are carried
        // as raw wire words by the shared projection.
        self.words[GRAVITY_WORD] = self.words[GRAVITY_WORD] as i16 as i32 as u32;
        let ground = player.movement.ground;
        let mut context = PlayerContext::default();
        self.projection.apply(
            &self.words,
            player,
            &mut context,
            ResourceNamespace::Module(module),
            |_| None,
        );
        if player.movement.grounded {
            player.movement.ground = ground;
        }
        if self.layout.wide {
            player.movement.ladder = self.words[PM_FLAGS_WORD] & 128 != 0;
            player.movement.view_height = f32::from(memory.read_field(address + 48, Byte)? as i8);
        }
        Ok(())
    }
    pub fn write_state(
        &mut self,
        memory: &mut ModuleMemory<'_>,
        address: u64,
        module: ModuleId,
        player: &PlayerState,
    ) -> Result<(), CallError> {
        let context = PlayerContext {
            namespace: Some(ResourceNamespace::Module(module)),
            gravity: qa_movement::effective_tuning(player.movement_rules, player.movement.tuning)
                .gravity,
            ..Default::default()
        };
        self.projection
            .reduce(player, &context, &mut self.words, |_| None, |_| None);
        let clear_flags = self.layout.wide
            && matches!(
                player.movement.mode,
                qa_core::primitives::MovementMode::Spectator
                    | qa_core::primitives::MovementMode::Noclip
            );
        if clear_flags {
            self.words[PM_FLAGS_WORD] &= self.projection.movement_flag_mask();
        }
        if self.layout.wide {
            self.words[PM_FLAGS_WORD] =
                (self.words[PM_FLAGS_WORD] & !128) | (u32::from(player.movement.ladder) << 7);
            memory.write_field(
                address + 48,
                Byte,
                player.movement.view_height as i8 as u8 as u32,
            )?;
        }
        for (&word, &(offset, width)) in self.words.iter().zip(&self.layout.state) {
            memory.write_field(address + offset as u64, width, word)?;
        }
        Ok(())
    }
    pub fn command(
        &self,
        memory: &ModuleMemory<'_>,
        address: u64,
        server_time: i32,
    ) -> Result<UserCmd, CallError> {
        self.client
            .read_command(memory, address + self.layout.command as u64, server_time)
    }
    pub fn import_client(
        &mut self,
        memory: &ModuleMemory<'_>,
        pointer: u64,
        module: ModuleId,
        player: &mut PlayerState,
    ) -> Result<(), CallError> {
        self.read_state(memory, pointer, module, player)?;
        self.client
            .import_presentation(memory, pointer, module, player)
    }
}
