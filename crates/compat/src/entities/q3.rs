//! Q3's public game-memory registration feeds the existing entity projector.
use super::*;

pub(crate) const ENTITY_NONE: i32 = 1023;
pub(crate) const ENTITY_WORLD: i32 = ENTITY_NONE - 1;
const ENTITY_BYTES: usize = 516;
const PLAYER_BYTES: usize = 468;
static APPEARANCE: qa_network::projection::EntityProjection =
    qa_network::projection::EntityProjection::load_native_q3();

#[derive(Clone, Copy)]
pub(super) struct GameData {
    pub entities: Entities,
    pub players: u64,
    pub player_stride: u64,
    pub player_count: u32,
}
impl GameData {
    pub fn validate(self, memory: &ModuleMemory<'_>) -> Result<(), CallError> {
        if self.entities.count > ENTITY_NONE as u32 + 1
            || self.entities.stride < ENTITY_BYTES as u64
            || self.player_count > 64
            || self.player_stride < PLAYER_BYTES as u64
        {
            return Err(CallError::Entity);
        }
        for (address, stride, count) in [
            (
                self.entities.address,
                self.entities.stride,
                self.entities.count,
            ),
            (self.players, self.player_stride, self.player_count),
        ] {
            let bytes = stride
                .checked_mul(u64::from(count))
                .and_then(|bytes| usize::try_from(bytes).ok())
                .ok_or(CallError::Memory)?;
            memory.read(address, bytes)?;
        }
        Ok(())
    }
}

pub(super) const LAYOUT: EntityLayout = EntityLayout {
    source: RuleSetId::Quake3,
    bytes: ENTITY_BYTES,
    in_use: None,
    linked: Some((416, false)),
    link_count: 420,
    flags: 424,
    snapshot_exclusion_mask: 1,
    player_flag: 0,
    projectile_flag: 0,
    capsule_flag: 0x400,
    mins: 436,
    maxs: 448,
    abs_min: 464,
    abs_max: 476,
    size: None,
    solid: (432, false),
    owner: 512,
    owner_number: true,
    origin: 488,
    angles: 500,
    contents: Some(460),
    world_zero: false,
    number: Some(0),
    contents_pose: Some((
        crate::players::q3_entity_offset(24),
        crate::players::q3_entity_offset(37),
    )),
    area: None,
    area2: None,
    clusters: None,
    network_solid: crate::players::q3_entity_offset(27),
    solid_codec: SolidCodec::QUAKE3,
    appearance: AppearanceImport::Projected(&APPEARANCE),
    model_rules: ModelRules {
        rotation: qa_core::primitives::ModelRotation::TransposeBasis,
        link_bounds: qa_core::primitives::RotatedLinkBounds::RadiusCube,
    },
};

impl EntityProjection {
    pub(crate) fn registered_players(&self) -> Option<Entities> {
        let EntityMemory::Registered(Some(data)) = self.memory else {
            return None;
        };
        Some(Entities {
            address: data.players,
            stride: data.player_stride,
            count: data.player_count,
            capacity: data.player_count,
            server_flags: 0,
        })
    }
    pub fn load_q3() -> Self {
        let mut projection = Self::load(
            0,
            false,
            LAYOUT,
            Default::default(),
            ENTITY_NONE as usize + 1,
        );
        projection.memory = EntityMemory::Registered(None);
        projection
    }
    pub(crate) fn register_game_data(
        &mut self,
        memory: &ModuleMemory<'_>,
        entities: Entities,
        players: u64,
        player_stride: u64,
        player_count: u32,
    ) -> Result<(), CallError> {
        let EntityMemory::Registered(_) = self.memory else {
            return Err(CallError::Entity);
        };
        let data = GameData {
            entities,
            players,
            player_stride,
            player_count,
        };
        data.validate(memory)?;
        self.memory = EntityMemory::Registered(Some(data));
        Ok(())
    }
}
