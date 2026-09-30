//! Voice English tutor (P5 dev spec): push-to-talk → whisper → local checks or the LLM tutor →
//! sentences for the speech queue in the webview.

pub mod capture;
pub mod intents;
pub mod reply_stream;
pub mod review;
pub mod session;
pub mod similarity;
pub mod stt;
pub mod tutor;
