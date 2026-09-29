//! Protocol version quirks across families.
//!
//! Donor provenance: `src/network/q2/constants.ts`
//! (`PROTOCOL_VERSION_*`), `src/network/q3/admission.ts` (protocol 68
//! gate), `src/network/q1/constants.ts` (`svc_updatestat`) and
//! `src/network/q1/qw-constants.ts` (`svc_updatestat`,
//! `svc_updatestatlong`). Wire version constants live in `qa-net`; this
//! module owns the capability predicates routers need.

use qa_world::client::ClientFamily;

/// Quake III accepted protocol (others are rejected at admission).
pub const Q3_ACCEPTED_PROTOCOL: u32 = 68;
/// R1Q2 revision adding the extended user command.
pub const R1Q2_UCMD_REVISION: u32 = 1904;
/// R1Q2 revision adding long solid encoding.
pub const R1Q2_LONG_SOLID_REVISION: u32 = 1905;
/// Q2Pro revision adding the server-state block.
pub const Q2PRO_SERVER_STATE_REVISION: u32 = 1019;
/// NetQuake `svc_updatestat`: `[byte] [long]`.
pub const NQ_SVC_UPDATESTAT: u8 = 3;
/// QuakeWorld `svc_updatestat`: `[byte] [byte]`.
pub const QW_SVC_UPDATESTAT: u8 = 3;
/// QuakeWorld `svc_updatestatlong`: `[byte] [long]`.
pub const QW_SVC_UPDATESTATLONG: u8 = 38;

/// Whether a server protocol version passes Q3 admission.
#[must_use]
pub const fn q3_accepts_protocol(version: u32) -> bool {
    version == Q3_ACCEPTED_PROTOCOL
}

/// Whether an R1Q2 revision carries the extended user command.
#[must_use]
pub const fn r1q2_supports_ucmd(revision: u32) -> bool {
    revision >= R1Q2_UCMD_REVISION
}

/// Whether an R1Q2 revision carries long solid encoding.
#[must_use]
pub const fn r1q2_supports_long_solid(revision: u32) -> bool {
    revision >= R1Q2_LONG_SOLID_REVISION
}

/// Whether a Q2Pro revision carries the server-state block.
#[must_use]
pub const fn q2pro_has_server_state(revision: u32) -> bool {
    revision >= Q2PRO_SERVER_STATE_REVISION
}

/// Q1 stat value width selected by family and message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1StatWidth {
    /// Single byte (QuakeWorld `svc_updatestat`).
    Byte,
    /// Four-byte long (NetQuake, QuakeWorld `svc_updatestatlong`).
    Long,
}

/// Stat value width for a Q1 status message, if the pair is valid.
#[must_use]
pub const fn q1_stat_width(family: ClientFamily, svc: u8) -> Option<Q1StatWidth> {
    match family {
        ClientFamily::Q1Netquake => {
            if svc == NQ_SVC_UPDATESTAT {
                Some(Q1StatWidth::Long)
            } else {
                None
            }
        }
        ClientFamily::Q1Quakeworld => {
            if svc == QW_SVC_UPDATESTAT {
                Some(Q1StatWidth::Byte)
            } else if svc == QW_SVC_UPDATESTATLONG {
                Some(Q1StatWidth::Long)
            } else {
                None
            }
        }
        ClientFamily::Q2Classic | ClientFamily::Q2Rerelease | ClientFamily::Q3 => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_gates_match_donor_thresholds() {
        assert!(q3_accepts_protocol(68));
        assert!(!q3_accepts_protocol(67));
        assert!(!r1q2_supports_ucmd(1903));
        assert!(r1q2_supports_ucmd(1904));
        assert!(!r1q2_supports_long_solid(1904));
        assert!(r1q2_supports_long_solid(1905));
        assert!(!q2pro_has_server_state(1018));
        assert!(q2pro_has_server_state(1019));
    }

    #[test]
    fn q1_stat_width_selects_by_family_and_svc() {
        assert_eq!(q1_stat_width(ClientFamily::Q1Netquake, 3), Some(Q1StatWidth::Long));
        assert_eq!(q1_stat_width(ClientFamily::Q1Quakeworld, 3), Some(Q1StatWidth::Byte));
        assert_eq!(q1_stat_width(ClientFamily::Q1Quakeworld, 38), Some(Q1StatWidth::Long));
        assert_eq!(q1_stat_width(ClientFamily::Q1Netquake, 38), None);
        assert_eq!(q1_stat_width(ClientFamily::Q3, 3), None);
    }
}
