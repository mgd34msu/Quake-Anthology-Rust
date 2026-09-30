//! Quake III client cvar initialization.
//!
//! Port of `src/app/bootstrap/q3-client/userinfo.ts`
//! (`initializeQ3ClientCvars`). Registration order, defaults, and flag words
//! match the donor exactly; flags reuse
//! [`qa_core::cvar::flags`](qa_core::cvar::flags).

use qa_core::cvar::{flags, CvarError, CvarRegistry};
use thiserror::Error;

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
pub fn initialize_q3_client_cvars(
    cvars: &mut CvarRegistry,
    identity: &Q3ClientIdentity,
) -> Result<(), Q3UserinfoError> {
    cvars.register("cl_timeNudge", "0", flags::TEMPORARY)?;
    cvars.register("rate", "25000", flags::ARCHIVE | flags::USER_INFO)?;
    cvars.register("cl_maxpackets", "30", flags::ARCHIVE)?;
    cvars.register("cl_packetdup", "1", flags::ARCHIVE)?;
    cvars.register("snaps", "20", flags::ARCHIVE | flags::USER_INFO)?;
    cvars.register(
        "name",
        &identity.name,
        flags::ARCHIVE | flags::USER_INFO,
    )?;
    for name in ["model", "headmodel", "team_model", "team_headmodel"] {
        cvars.register(
            name,
            &format!("{}/default", identity.model),
            flags::ARCHIVE | flags::USER_INFO,
        )?;
    }
    for (name, value) in [
        ("color1", "4"),
        ("color2", "5"),
        ("sex", "male"),
        ("cl_anonymous", "0"),
        ("cg_predictItems", "1"),
    ] {
        cvars.register(name, value, flags::ARCHIVE | flags::USER_INFO)?;
    }
    cvars.register("teamtask", "0", flags::USER_INFO)?;
    cvars.register("password", "", flags::USER_INFO)?;
    cvars.register("handicap", "100", flags::ARCHIVE | flags::USER_INFO)?;
    cvars.register("cl_maxPing", "800", flags::ARCHIVE)?;
    cvars.register("cl_serverStatusResendTime", "750", flags::NONE)?;
    cvars.register("sv_master1", "master.quake3arena.com", flags::NONE)?;
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
        let archived = ["rate", "snaps", "color1", "color2", "sex", "cl_anonymous",
            "cg_predictItems", "handicap"];
        for name in archived {
            let snapshot = cvars.get(name).expect("cvar is registered");
            assert_eq!(
                snapshot.flags,
                flags::ARCHIVE | flags::USER_INFO,
                "{name} flags"
            );
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
        let resend = cvars
            .get("cl_serverStatusResendTime")
            .expect("resend is registered");
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
