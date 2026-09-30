//! Voice English tutor (P5 dev spec): push-to-talk → whisper → local checks or the LLM tutor →
//! sentences for the speech queue in the webview.

pub mod intents;
pub mod reply_stream;
pub mod similarity;
pub mod tutor;
