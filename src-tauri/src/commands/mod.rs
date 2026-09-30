//! Thin IPC layer: parse args → call a service → map errors. See docs/dev/P1 §6.

pub mod ai;
pub mod news;
pub mod onboarding;
pub mod reader;
pub mod settings;
pub mod shell;
pub mod voice;
pub mod words;

use crate::error::AppError;

pub type CmdResult<T> = Result<T, AppError>;
