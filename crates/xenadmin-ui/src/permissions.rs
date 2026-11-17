//! The session permission assessment every review uses.
//!
//! This is the logic of the Rune `session_permissions_view.rs`: `assess`,
//! `check`, and the text of the review footer. The rule it encodes, relied
//! on everywhere: **only a known denial disables or refuses; an unknown
//! permission lets the action proceed to the host's own check; a grant
//! never overrides object ineligibility.**
//!
//! In a review:
//!
//! ```ignore
//! let permission = permissions::assess(m, &requirements);
//! Surface::dialog("host-maintenance", "Enter maintenance mode")
//!     .footer_note(permissions::footer_note(&permission))
//!     .footer([
//!         cancel,
//!         button("host-confirm", "Confirm", confirm)
//!             .primary()
//!             .disabled_opt(stale)
//!             .disabled_opt(permissions::denial(&permission))
//!             .disabled_if(m.demo, "Demo mode cannot change infrastructure."),
//!     ])
//! ```
//!
//! and the handler checks again with [`check`] before it submits.

use xenadmin_core::{
    PermissionAssessment, PermissionDecision, PermissionRequirements, PoolId, PowerAction,
};

use crate::model::Model;
use crate::surface::{Text, caption};

/// The key of the permission line of a review footer.
pub const REVIEW_KEY: &str = "review-permission-explanation";

/// The key of the permission line beside the power commands.
pub const SESSION_KEY: &str = "session-permission-explanation";

/// What the scope session's permissions say about a set of requirements.
/// Without a snapshot the permissions are unknown, which never blocks.
pub fn assess(m: &Model, requirements: &PermissionRequirements) -> PermissionAssessment {
    requirements.assess(
        &m.snapshot
            .as_ref()
            .map(|s| s.session_permissions())
            .unwrap_or_default(),
    )
}

/// [`assess`] against the session through which a pool is reached, for a
/// review whose target is in another pool than the scope session's.
pub fn assess_for(
    m: &Model,
    pool: PoolId,
    requirements: &PermissionRequirements,
) -> PermissionAssessment {
    requirements.assess(
        &m.snapshot_for(pool)
            .map(|s| s.session_permissions())
            .unwrap_or_default(),
    )
}

/// The submit-time gate: a known denial puts its explanation in `m.error`
/// and returns false; granted or unknown returns true.
pub fn check(m: &mut Model, requirements: &PermissionRequirements) -> bool {
    let assessment = assess(m, requirements);
    if assessment.decision().is_denied() {
        m.error = Some(assessment.explanation());
        false
    } else {
        true
    }
}

/// The reason a review's confirm button is disabled: the explanation of a
/// known denial. `None` when the permission is granted or unknown.
pub fn denial(assessment: &PermissionAssessment) -> Option<String> {
    assessment
        .decision()
        .is_denied()
        .then(|| assessment.explanation())
}

/// The permission line of a review footer (`Surface::footer_note`): the
/// assessment's explanation, whatever the decision.
pub fn footer_note(assessment: &PermissionAssessment) -> Text {
    caption(assessment.explanation()).key(REVIEW_KEY)
}

/// The scope session's decision about one power action.
pub fn power(m: &Model, action: PowerAction) -> Option<PermissionDecision> {
    m.snapshot
        .as_ref()
        .map(|snapshot| snapshot.power_permission(action))
}

/// The reason a power command is disabled by the session's permissions:
/// the explanation of a known denial, else `None`.
pub fn power_denial(permission: Option<PermissionDecision>) -> Option<String> {
    permission
        .filter(|permission| permission.is_denied())
        .map(|permission| permission.explanation().to_owned())
}

/// The permission line beside the power commands.
pub fn notice(permission: PermissionDecision) -> Text {
    caption(permission.explanation()).key(SESSION_KEY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenadmin_core::PermissionUnknown;

    #[test]
    fn without_a_session_the_permission_is_unknown_and_never_blocks() {
        let mut m = Model::demo(3, 48);
        let requirements = xenadmin_core::HostOperation::ExitMaintenance.permission_requirements();
        let assessment = assess(&m, &requirements);
        assert!(!assessment.decision().is_denied());
        assert_eq!(denial(&assessment), None);
        assert!(check(&mut m, &requirements));
        assert!(m.error.is_none());
        let note = footer_note(&assessment);
        assert_eq!(note.key.as_deref(), Some(REVIEW_KEY));
        assert_eq!(note.text, assessment.explanation());
        let pool = m.inventory.clone().unwrap().pool_id;
        assert_eq!(
            assess_for(&m, pool, &requirements).explanation(),
            assessment.explanation()
        );
        assert_eq!(power(&m, PowerAction::Start), None);
    }

    // Rune: session_permissions_view.rs
    // `known_denial_disables_power_without_hiding_its_reason_at_both_widths` and
    // `unknown_permissions_allow_review_but_grants_never_override_object_ineligibility`,
    // the rule without the buttons.
    #[test]
    fn only_a_known_denial_gives_a_reason_to_disable() {
        assert_eq!(
            power_denial(Some(PermissionDecision::Denied)).as_deref(),
            Some(PermissionDecision::Denied.explanation())
        );
        assert_eq!(power_denial(Some(PermissionDecision::Granted)), None);
        assert_eq!(
            power_denial(Some(PermissionDecision::Unknown(
                PermissionUnknown::Unsupported
            ))),
            None
        );
        assert_eq!(power_denial(None), None);
        let text = notice(PermissionDecision::Denied);
        assert_eq!(text.key.as_deref(), Some(SESSION_KEY));
        assert!(text.text.contains("does not grant permission"));
    }
}
