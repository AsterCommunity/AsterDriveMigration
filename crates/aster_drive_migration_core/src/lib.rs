//! Schema-independent migration contracts, domain values, and state rules.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::unreachable,
        clippy::expect_used,
        clippy::panic,
        clippy::unimplemented,
        clippy::todo
    )
)]

pub mod conversion;
pub mod domain;
pub mod state;

pub use conversion::{Conversion, ConversionContext, SkipReason, SourceConverter};
pub use domain::*;
pub use state::{RunStatus, StageId, StagePlan, StateError};
