//! Pure migration-run state and ordered-stage rules.

use std::borrow::Cow;
use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Running,
    Failed,
    ValidationFailed,
    Aborted,
    Completed,
}

impl RunStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Failed => "failed",
            Self::ValidationFailed => "validation_failed",
            Self::Aborted => "aborted",
            Self::Completed => "completed",
        }
    }

    pub fn parse(value: &str) -> Result<Self, StateError> {
        match value {
            "running" => Ok(Self::Running),
            "failed" => Ok(Self::Failed),
            "validation_failed" => Ok(Self::ValidationFailed),
            "aborted" => Ok(Self::Aborted),
            "completed" => Ok(Self::Completed),
            _ => Err(StateError::UnknownStatus(value.to_string())),
        }
    }

    #[must_use]
    pub const fn can_resume(self) -> bool {
        matches!(self, Self::Running | Self::Failed | Self::ValidationFailed)
    }

    #[must_use]
    pub const fn can_abort(self) -> bool {
        matches!(self, Self::Running | Self::Failed | Self::ValidationFailed)
    }

    #[must_use]
    pub const fn can_cleanup(self) -> bool {
        matches!(self, Self::Completed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StageId(Cow<'static, str>);

impl StageId {
    #[must_use]
    pub const fn borrowed(value: &'static str) -> Self {
        Self(Cow::Borrowed(value))
    }

    pub fn owned(value: impl Into<String>) -> Result<Self, StateError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(StateError::EmptyStage);
        }
        Ok(Self(Cow::Owned(value)))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagePlan {
    stages: Vec<StageId>,
}

impl StagePlan {
    pub fn new(stages: impl IntoIterator<Item = StageId>) -> Result<Self, StateError> {
        let stages = stages.into_iter().collect::<Vec<_>>();
        if stages.is_empty() {
            return Err(StateError::EmptyPlan);
        }
        for (index, stage) in stages.iter().enumerate() {
            if stage.as_str().trim().is_empty() {
                return Err(StateError::EmptyStage);
            }
            if stages[..index].contains(stage) {
                return Err(StateError::DuplicateStage(stage.as_str().to_string()));
            }
        }
        Ok(Self { stages })
    }

    #[must_use]
    pub fn stages(&self) -> &[StageId] {
        &self.stages
    }

    pub fn should_run_after(
        &self,
        stage: &StageId,
        last_completed: Option<&str>,
    ) -> Result<bool, StateError> {
        let current = self.index_of(stage.as_str())?;
        let Some(last_completed) = last_completed else {
            return Ok(true);
        };
        Ok(current > self.index_of(last_completed)?)
    }

    pub fn next_after(&self, last_completed: Option<&str>) -> Result<Option<&StageId>, StateError> {
        match last_completed {
            None => Ok(self.stages.first()),
            Some(stage) => Ok(self.stages.get(self.index_of(stage)? + 1)),
        }
    }

    fn index_of(&self, value: &str) -> Result<usize, StateError> {
        self.stages
            .iter()
            .position(|stage| stage.as_str() == value)
            .ok_or_else(|| StateError::UnknownStage(value.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateError {
    UnknownStatus(String),
    UnknownStage(String),
    DuplicateStage(String),
    EmptyStage,
    EmptyPlan,
}

impl fmt::Display for StateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownStatus(value) => write!(formatter, "unknown migration status {value}"),
            Self::UnknownStage(value) => write!(formatter, "unknown migration stage {value}"),
            Self::DuplicateStage(value) => write!(formatter, "duplicate migration stage {value}"),
            Self::EmptyStage => formatter.write_str("migration stage name must not be empty"),
            Self::EmptyPlan => formatter.write_str("migration stage plan must not be empty"),
        }
    }
}

impl Error for StateError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> StagePlan {
        StagePlan::new([
            StageId::borrowed("users"),
            StageId::borrowed("files"),
            StageId::borrowed("shares"),
        ])
        .expect("valid plan")
    }

    #[test]
    fn status_round_trips_and_rejects_unknown_values() {
        for status in [
            RunStatus::Running,
            RunStatus::Failed,
            RunStatus::ValidationFailed,
            RunStatus::Aborted,
            RunStatus::Completed,
        ] {
            assert_eq!(RunStatus::parse(status.as_str()), Ok(status));
        }
        assert_eq!(
            RunStatus::parse("paused"),
            Err(StateError::UnknownStatus("paused".to_string()))
        );
    }

    #[test]
    fn state_operations_observe_terminal_boundaries() {
        assert!(RunStatus::Running.can_resume());
        assert!(RunStatus::Failed.can_resume());
        assert!(RunStatus::ValidationFailed.can_resume());
        assert!(!RunStatus::Aborted.can_resume());
        assert!(!RunStatus::Completed.can_resume());
        assert!(RunStatus::ValidationFailed.can_abort());
        assert!(!RunStatus::Completed.can_abort());
        assert!(RunStatus::Completed.can_cleanup());
        assert!(!RunStatus::Failed.can_cleanup());
    }

    #[test]
    fn stage_plan_selects_next_stage_at_every_boundary() -> Result<(), StateError> {
        let plan = plan();
        assert_eq!(
            plan.next_after(None)
                .map(|stage| stage.map(StageId::as_str))?,
            Some("users")
        );
        assert_eq!(
            plan.next_after(Some("users"))
                .map(|stage| stage.map(StageId::as_str))?,
            Some("files")
        );
        assert_eq!(plan.next_after(Some("shares"))?, None);
        assert!(!plan.should_run_after(&StageId::borrowed("users"), Some("users"))?);
        assert!(plan.should_run_after(&StageId::borrowed("shares"), Some("users"))?);
        Ok(())
    }

    #[test]
    fn stage_plan_rejects_invalid_and_unknown_stages() {
        assert_eq!(StagePlan::new([]), Err(StateError::EmptyPlan));
        assert_eq!(
            StagePlan::new([StageId::borrowed("users"), StageId::borrowed("users")]),
            Err(StateError::DuplicateStage("users".to_string()))
        );
        assert_eq!(
            plan().next_after(Some("missing")),
            Err(StateError::UnknownStage("missing".to_string()))
        );
        assert_eq!(StageId::owned("  "), Err(StateError::EmptyStage));
    }
}
