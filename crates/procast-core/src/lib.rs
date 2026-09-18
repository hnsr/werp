//! Reusable media and casting backend, independent of terminal presentation.

pub mod cast;
pub mod discovery;
mod error;
pub mod media;
mod process;
pub mod serve;
pub mod session;
pub mod subtitles;
pub mod transcode;

pub use error::ProcastError;
pub use tokio_util::sync::CancellationToken;
