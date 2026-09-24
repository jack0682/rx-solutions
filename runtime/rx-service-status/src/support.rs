//! Typed ownership refusal from the shared Repository port; no message parsing.
//! This classifies a refusal, never establishes the identity of a lock holder.
use serde::Serialize;
#[derive(Debug, Serialize)]
pub struct SupportRefusal {
    pub schema: &'static str,
    pub condition: &'static str,
    pub decision: &'static str,
    pub owner_identity: &'static str,
    pub detail: String,
    pub recovery: &'static str,
}
pub fn storage_ownership_refusal(
    error: &(dyn std::error::Error + 'static),
) -> Option<SupportRefusal> {
    let mut current = Some(error);
    while let Some(error) = current {
        if let Some(rx_ports::StoreError::Ownership(ownership)) =
            error.downcast_ref::<rx_ports::StoreError>()
        {
            let condition = match ownership.kind {
                rx_ports::OwnershipFailure::Contended => "storage/exclusive-writer-not-established",
                rx_ports::OwnershipFailure::Acquire => "storage/ownership-acquisition-failed",
                rx_ports::OwnershipFailure::ForeignProcess => {
                    "storage/inherited-repository-refused"
                }
                rx_ports::OwnershipFailure::ConnectionClose => {
                    "storage/connection-close-unconfirmed"
                }
                rx_ports::OwnershipFailure::Release => "storage/ownership-release-unconfirmed",
            };
            let recovery = match ownership.kind {
                rx_ports::OwnershipFailure::Contended => {
                    "OBSERVE_OWNER_OR_INHERITED_DESCRIPTION_RELEASE_THEN_EXPLICIT_OPEN; ABRUPT_LOSS_CAN_RETAIN_LOCK_UNTIL_INHERITED_DESCRIPTION_CLOSES; NO_AUTOMATIC_RETRY_OR_LOCK_FILE_DELETION"
                }
                rx_ports::OwnershipFailure::Acquire => {
                    "REVIEW_ACQUISITION_ERROR_BEFORE_EXPLICIT_OPEN; NO_OWNER_INFERRED"
                }
                rx_ports::OwnershipFailure::ForeignProcess => {
                    "DO_NOT_REUSE_INHERITED_REPOSITORY; EXEC_OR_EXIT_TO_DISCARD_INHERITED_SQLITE_STATE"
                }
                rx_ports::OwnershipFailure::ConnectionClose => {
                    "CONNECTION_AND_LOCK_RETAINED_FOR_PROCESS_LIFETIME; INHERITED_COPIES_MAY_EXTEND_LOCK_LIFETIME; NO_AUTOMATIC_RETRY"
                }
                rx_ports::OwnershipFailure::Release => {
                    "RELEASE_UNCONFIRMED; DESCRIPTOR_RETAINED_FOR_PROCESS_LIFETIME; NO_AUTOMATIC_RETRY"
                }
            };
            return Some(SupportRefusal {
                schema: "rx.support-refusal.v1",
                condition,
                decision: "REFUSED",
                owner_identity: "NOT_ESTABLISHED",
                detail: ownership.detail.clone(),
                recovery,
            });
        }
        current = error.source();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_text_is_not_a_typed_ownership_failure() {
        let legacy =
            rx_ports::StoreError::Unavailable("another Runtime owns this store: fake text".into());
        assert!(storage_ownership_refusal(&legacy).is_none());
        let typed = rx_ports::StoreError::Ownership(rx_ports::OwnershipError {
            kind: rx_ports::OwnershipFailure::Contended,
            detail: "held".into(),
        });
        let refusal = storage_ownership_refusal(&typed).unwrap();
        assert_eq!(
            refusal.condition,
            "storage/exclusive-writer-not-established"
        );
        assert_eq!(refusal.owner_identity, "NOT_ESTABLISHED");
        assert!(refusal.recovery.contains("ABRUPT_LOSS"));
    }
}
