//! Diagnostic adapter for the pinned SDK's legacy untyped storage error.
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
        // The SDK currently reports every try_lock failure with this prefix.
        // Keep this compatibility adapter pinned and tested until its owning
        // platform supplies a typed error. Other storage failures are not guessed.
        if let Some((_, detail)) = error
            .to_string()
            .split_once("another Runtime owns this store: ")
        {
            return Some(SupportRefusal {
                schema: "rx.support-refusal.v1",
                condition: "storage/exclusive-writer-not-established",
                decision: "REFUSED",
                owner_identity: "NOT_ESTABLISHED",
                detail: detail.into(),
                recovery: "OBSERVE_EXISTING_OWNER_OR_INHERITED_DESCRIPTOR_RELEASE_THEN_EXPLICITLY_RETRY; NO_AUTOMATIC_RETRY_OR_LOCK_FILE_DELETION",
            });
        }
        current = error.source();
    }
    None
}
