//! ROS JTC result meanings, supplied as data to the transport-independent platform.
use super::Profile;
use crate::Result;
use rx_domain::types::{Integer, Name};
use rx_process_contract::native_outcome::{
    NativeConclusion, NativeOutcomeCase, NativeOutcomeTable,
};

impl Profile {
    pub fn outcome_table(&self) -> Result<NativeOutcomeTable> {
        let case = |schema: &str, codes: &[i64], conclusion| NativeOutcomeCase {
            status_schema: Name::new(schema).expect("fixed schema"),
            statuses: codes.iter().copied().map(Integer).collect(),
            conclusion,
        };
        Ok(NativeOutcomeTable {
            schema: Name::new("rx.native-outcome-table.v1").expect("fixed schema"),
            profile_digest: self.digest()?,
            completion_rule: Name::new("rx.ros-jtc.terminal-result.v1").expect("fixed rule"),
            cases: vec![
                case("rx.ros-jtc.succeeded.v1", &[0], NativeConclusion::Succeeded),
                case(
                    "rx.ros-jtc.canceled.v1",
                    &[-5, -4, -3, -2, -1, 0],
                    NativeConclusion::Canceled,
                ),
                case(
                    "rx.ros-jtc.aborted.v1",
                    &[-5, -4, -3, -2, -1, 0],
                    NativeConclusion::Failed,
                ),
                // The native call was entered. Rejection is not a pre-send tombstone.
                case(
                    "rx.ros-jtc.goal-rejected.v1",
                    &[0],
                    NativeConclusion::Failed,
                ),
            ],
        })
    }
}
