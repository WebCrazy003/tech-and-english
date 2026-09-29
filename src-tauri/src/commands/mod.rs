//! Thin IPC layer: parse args → call a service → map errors. See docs/dev/P1 §6.

pub mod news;
pub mod onboarding;
pub mod settings;
pub mod shell;

use crate::error::AppError;

pub type CmdResult<T> = Result<T, AppError>;
