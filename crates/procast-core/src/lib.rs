//! Reusable media and casting backend, independent of terminal presentation.

mod error;
pub mod media;
mod process;

pub use error::ProcastError;
pub use tokio_util::sync::CancellationToken;
