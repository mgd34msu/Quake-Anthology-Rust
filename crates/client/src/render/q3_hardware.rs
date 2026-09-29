//! Q3 GL driver classification (`linux_glimp.c` startup).
//!
//! Donor provenance: `src/render/q3-hardware.ts`. The renderer string names
//! the detected board; only Rage Pro keeps its own driver path, everything
//! else shares the generic ICD path.

/// Detected Q3-era GL hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q3Hardware {
    /// Default ICD path.
    Generic,
    /// 3Dfx Voodoo/Banshee.
    Hardware3dfx,
    /// NVIDIA RIVA 128.
    Riva128,
    /// ATI Rage Pro.
    RagePro,
    /// 3Dlabs Permedia 2.
    Permedia2,
}

/// Effective driver path after collapsing every non-Rage-Pro board onto the
/// generic ICD path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveHardware {
    /// Effective board: [`Q3Hardware::RagePro`] or [`Q3Hardware::Generic`].
    pub hardware: Q3Hardware,
    /// Driver type name: `"rage-pro"` or `"generic"`.
    pub hardware_type: &'static str,
}

/// Classify a GL renderer string, matching the donor's case-insensitive
/// substring checks in order.
#[must_use]
pub fn q3_hardware(renderer: &str) -> Q3Hardware {
    let name = renderer.to_lowercase();
    if name.contains("banshee") || name.contains("voodoo_graphics") {
        Q3Hardware::Hardware3dfx
    } else if name.contains("rage pro") || name.contains("ragepro") {
        Q3Hardware::RagePro
    } else if name.contains("permedia2") {
        Q3Hardware::Permedia2
    } else if name.contains("riva 128") {
        Q3Hardware::Riva128
    } else {
        Q3Hardware::Generic
    }
}

/// Numeric board id: generic 0, 3dfx 1, RIVA 128 2, Rage Pro 3, Permedia 2 4.
#[must_use]
pub const fn q3_hardware_number(hardware: Q3Hardware) -> u32 {
    match hardware {
        Q3Hardware::Generic => 0,
        Q3Hardware::Hardware3dfx => 1,
        Q3Hardware::Riva128 => 2,
        Q3Hardware::RagePro => 3,
        Q3Hardware::Permedia2 => 4,
    }
}

/// Collapse a detected board onto its effective driver path.
#[must_use]
pub const fn effective_hardware(hardware: Q3Hardware) -> EffectiveHardware {
    match hardware {
        Q3Hardware::RagePro => EffectiveHardware {
            hardware: Q3Hardware::RagePro,
            hardware_type: "rage-pro",
        },
        _ => EffectiveHardware {
            hardware: Q3Hardware::Generic,
            hardware_type: "generic",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_boards() {
        assert_eq!(q3_hardware("3Dfx/Voodoo_Graphics"), Q3Hardware::Hardware3dfx);
        assert_eq!(q3_hardware("STB Banshee 16MB"), Q3Hardware::Hardware3dfx);
        assert_eq!(q3_hardware("ATI Rage Pro AGP 2X"), Q3Hardware::RagePro);
        assert_eq!(q3_hardware("ragepro gl"), Q3Hardware::RagePro);
        assert_eq!(q3_hardware("3DLabs Permedia2"), Q3Hardware::Permedia2);
        assert_eq!(q3_hardware("NVidia Riva 128"), Q3Hardware::Riva128);
    }

    #[test]
    fn unknown_renderers_are_generic() {
        assert_eq!(q3_hardware("Mesa DRI Intel"), Q3Hardware::Generic);
        assert_eq!(q3_hardware(""), Q3Hardware::Generic);
        assert_eq!(q3_hardware("RIVA TNT2"), Q3Hardware::Generic);
        assert_eq!(q3_hardware("Rage 128"), Q3Hardware::Generic);
    }

    #[test]
    fn check_order_prefers_earlier_boards() {
        assert_eq!(
            q3_hardware("banshee rage pro"),
            Q3Hardware::Hardware3dfx,
            "3dfx check runs before ragepro"
        );
        assert_eq!(
            q3_hardware("rage pro permedia2"),
            Q3Hardware::RagePro,
            "ragepro check runs before permedia2"
        );
    }

    #[test]
    fn numbers_match_donor_order() {
        assert_eq!(q3_hardware_number(Q3Hardware::Generic), 0);
        assert_eq!(q3_hardware_number(Q3Hardware::Hardware3dfx), 1);
        assert_eq!(q3_hardware_number(Q3Hardware::Riva128), 2);
        assert_eq!(q3_hardware_number(Q3Hardware::RagePro), 3);
        assert_eq!(q3_hardware_number(Q3Hardware::Permedia2), 4);
    }

    #[test]
    fn only_ragepro_survives_effective_mapping() {
        assert_eq!(
            effective_hardware(Q3Hardware::RagePro),
            EffectiveHardware {
                hardware: Q3Hardware::RagePro,
                hardware_type: "rage-pro",
            }
        );
        for hardware in [
            Q3Hardware::Generic,
            Q3Hardware::Hardware3dfx,
            Q3Hardware::Riva128,
            Q3Hardware::Permedia2,
        ] {
            assert_eq!(
                effective_hardware(hardware),
                EffectiveHardware {
                    hardware: Q3Hardware::Generic,
                    hardware_type: "generic",
                },
                "{hardware:?}"
            );
        }
    }
}
