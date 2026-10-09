//! Load-owned gameplay names and the common item, weapon and HUD bindings.
use qa_core::names::NameTable;
use qa_gameplay::registry::Registry;
use qa_ui::hud::HudBindings;

pub struct GameplayCatalog {
    pub names: NameTable,
    pub registry: Registry,
    pub hud: HudBindings,
}

impl GameplayCatalog {
    /// Map, module and precache names join the stock registry before IDs bind.
    /// The resulting table and its numeric bindings share one load lifetime.
    pub fn load<'a>(extra_names: impl IntoIterator<Item = &'a [u8]>) -> Result<Self, String> {
        let names = NameTable::load(
            extra_names
                .into_iter()
                .chain(Registry::names_needed().map(|name| -> &'a [u8] { name })),
        )
        .map_err(|e| format!("gameplay names: {e:?}"))?;
        let registry = Registry::load(&names).map_err(|e| format!("item registry: {e:?}"))?;
        let hud = HudBindings::load(&registry, &[]).ok_or("HUD value layout capacity")?;
        Ok(Self {
            names,
            registry,
            hud,
        })
    }
}
