//! Quake III client cvar initialization.
//!
//! Port of `src/app/bootstrap/q3-client/userinfo.ts`
//! (`initializeQ3ClientCvars`). Registration order, defaults, and flag words
//! match the donor exactly; flags reuse
//! [`qa_core::cvar::flags`](qa_core::cvar::flags).

use qa_core::cvar::{flags, CvarError, CvarRegistry};
use thiserror::Error;

use crate::bootstrap::player_userinfo::declare_userinfo_rows;

/// Quake III client identity for userinfo defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3ClientIdentity {
    /// Player name.
    pub name: String,
    /// Player model.
    pub model: String,
}

/// Error for Quake III client cvar initialization.
#[derive(Debug, Error)]
pub enum Q3UserinfoError {
    /// Cvar registration failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
}

/// Register Quake III client cvars (donor `initializeQ3ClientCvars`).
/// Registration order, defaults, and flag words match the donor exactly;
/// rows declare through the shared userinfo helper, which is a plain
/// registration on Quake III registries.
pub fn initialize_q3_client_cvars(
    cvars: &mut CvarRegistry,
    identity: &Q3ClientIdentity,
) -> Result<(), Q3UserinfoError> {
    let info_flags = flags::ARCHIVE | flags::USER_INFO;
    let model_default = format!("{}/default", identity.model);
    declare_userinfo_rows(
        cvars,
        [
            ("cl_timeNudge", "0", flags::TEMPORARY),
            ("rate", "25000", info_flags),
            ("cl_maxpackets", "30", flags::ARCHIVE),
            ("cl_packetdup", "1", flags::ARCHIVE),
            ("snaps", "20", info_flags),
            ("name", identity.name.as_str(), info_flags),
            ("model", model_default.as_str(), info_flags),
            ("headmodel", model_default.as_str(), info_flags),
            ("team_model", model_default.as_str(), info_flags),
            ("team_headmodel", model_default.as_str(), info_flags),
            ("color1", "4", info_flags),
            ("color2", "5", info_flags),
            ("sex", "male", info_flags),
            ("cl_anonymous", "0", info_flags),
            ("cg_predictItems", "1", info_flags),
            ("teamtask", "0", flags::USER_INFO),
            ("password", "", flags::USER_INFO),
            ("handicap", "100", info_flags),
            ("cl_maxPing", "800", flags::ARCHIVE),
            ("cl_serverStatusResendTime", "750", flags::NONE),
            ("sv_master1", "master.quake3arena.com", flags::NONE),
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;

    fn registry() -> CvarRegistry {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        initialize_q3_client_cvars(
            &mut cvars,
            &Q3ClientIdentity {
                name: "Player".to_string(),
                model: "sarge".to_string(),
            },
        )
        .unwrap();
        cvars
    }

    #[test]
    fn registers_identity_defaults() {
        let cvars = registry();
        let name = cvars.get("name").expect("name is registered");
        assert_eq!(name.value, "Player");
        assert_eq!(name.flags, flags::ARCHIVE | flags::USER_INFO);
        for model in ["model", "headmodel", "team_model", "team_headmodel"] {
            let snapshot = cvars.get(model).expect("model cvar is registered");
            assert_eq!(snapshot.value, "sarge/default");
            assert_eq!(snapshot.flags, flags::ARCHIVE | flags::USER_INFO);
        }
    }

    #[test]
    fn registers_network_and_identity_flags() {
        let cvars = registry();
        let archived = [
            "rate",
            "snaps",
            "color1",
            "color2",
            "sex",
            "cl_anonymous",
            "cg_predictItems",
            "handicap",
        ];
        for name in archived {
            let snapshot = cvars.get(name).expect("cvar is registered");
            assert_eq!(snapshot.flags, flags::ARCHIVE | flags::USER_INFO, "{name} flags");
        }
        for name in ["cl_maxpackets", "cl_packetdup", "cl_maxPing"] {
            let snapshot = cvars.get(name).expect("cvar is registered");
            assert_eq!(snapshot.flags, flags::ARCHIVE, "{name} flags");
        }
        for name in ["teamtask", "password"] {
            let snapshot = cvars.get(name).expect("cvar is registered");
            assert_eq!(snapshot.flags, flags::USER_INFO, "{name} flags");
        }
        let nudge = cvars.get("cl_timeNudge").expect("nudge is registered");
        assert_eq!(nudge.flags, flags::TEMPORARY);
        assert_eq!(cvars.get("rate").unwrap().value, "25000");
        assert_eq!(cvars.get("snaps").unwrap().value, "20");
        assert_eq!(cvars.get("password").unwrap().value, "");
    }

    #[test]
    fn registers_unflagged_locals() {
        let cvars = registry();
        let resend = cvars.get("cl_serverStatusResendTime").expect("resend is registered");
        assert_eq!(resend.value, "750");
        assert_eq!(resend.flags, flags::NONE);
        let master = cvars.get("sv_master1").expect("master is registered");
        assert_eq!(master.value, "master.quake3arena.com");
        assert_eq!(master.flags, flags::NONE);
    }

    #[test]
    fn cvar_errors_surface_transparently() {
        let error = Q3UserinfoError::from(CvarError::Domain("boom".to_string()));
        assert_eq!(error.to_string(), "boom");
    }
}
