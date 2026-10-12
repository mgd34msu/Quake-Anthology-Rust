use qa_core::primitives::{
    ClientId, EntityAppearance, ModelRef, ModuleId, NativeAppearanceTail, ResourceNamespace,
    RuleSetId, SoundRef, ThinkTime, Vec3,
};
use qa_world::entities::{AllocationPolicy, EntityTable};

fn appearance() -> EntityAppearance {
    EntityAppearance {
        source: RuleSetId::Quake2Rerelease,
        snapshot_excluded: true,
        models: [
            ModelRef::native(ResourceNamespace::Module(ModuleId(7)), 1),
            ModelRef::native(ResourceNamespace::Module(ModuleId(8)), 1),
            ModelRef::native(ResourceNamespace::Client(ClientId(7)), 1),
            ModelRef::native(ResourceNamespace::Client(ClientId(8)), u32::MAX),
        ],
        loop_sound: SoundRef::native(ResourceNamespace::Module(ModuleId(7)), u32::MAX),
        frame: i32::MIN,
        skin: i32::MAX,
        old_origin: Vec3([-1.25, 2.5, 3.75]),
        alpha: 0.5,
        scale: 2.0,
        native: NativeAppearanceTail {
            colormap: u32::MAX,
            effects: u64::MAX,
            render_flags: u32::MAX,
            event: u32::MAX,
            entity_type: u32::MAX,
            other_entity: u32::MAX,
            solid: u32::MAX,
            instance_bits: u8::MAX,
            loop_volume: 0.25,
            loop_attenuation: -1.0,
            owner: i32::MIN,
            old_frame: i32::MAX,
        },
    }
}

#[test]
fn a_reused_lifetime_cannot_retain_appearance_from_another_resource_namespace()
-> Result<(), &'static str> {
    let mut table = EntityTable::new(2, 1).map_err(|_| "table")?;
    let first = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(7),
            AllocationPolicy::EDICT,
        )
        .ok_or("allocation")?
        .id;
    let value = appearance();
    table.columns.appearance[first.slot as usize] = value;
    assert_eq!(table.columns.appearance[first.slot as usize], value);
    assert_ne!(value.models[0], value.models[1]);
    assert_ne!(value.models[0], value.models[2]);
    assert!(table.release(first, ThinkTime::Seconds(0.0)));
    let second = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(8),
            AllocationPolicy::EDICT,
        )
        .ok_or("replacement")?
        .id;
    assert_eq!(first.slot, second.slot);
    assert_ne!(first.generation, second.generation);
    assert_eq!(
        table.columns.appearance[second.slot as usize],
        EntityAppearance::default()
    );
    assert_eq!(table.columns.appearance[second.slot as usize].alpha, 1.0);
    assert_eq!(table.columns.appearance[second.slot as usize].scale, 1.0);
    Ok(())
}

#[test]
fn reconnecting_a_reserved_client_clears_every_appearance_field() -> Result<(), &'static str> {
    let mut table = EntityTable::new(2, 2).map_err(|_| "table")?;
    let first = table.id_at(1).ok_or("client")?;
    table.columns.appearance[first.slot as usize] = appearance();
    let second = table.reset_client(first).ok_or("reset")?;
    assert_ne!(first.generation, second.generation);
    assert!(table.resolve(first).is_none());
    assert_eq!(
        table.columns.appearance[second.slot as usize],
        EntityAppearance::default()
    );
    assert!(ModelRef::native(ResourceNamespace::Client(ClientId(0)), 0).is_none());
    Ok(())
}
