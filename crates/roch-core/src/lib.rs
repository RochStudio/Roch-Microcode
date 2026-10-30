//! Roch Microcode core engine.
//!
//! Copyright (C) 2026 Roch Studio. Licensed GPL-3.0-or-later.
//!
//! This crate is a behavior-preserving port of the original `RochBios.Core`
//! .NET library: Intel microcode discovery and checksums, FIT / FFS / FV
//! structure validation, the transfer builder, the version 2 package format,
//! the bundled independent verifiers (UEFIExtract and MCExtractor) and the USB
//! eligibility policy. Every rejection rule of the original is preserved.

pub mod builder;
pub mod cancel;
#[cfg(feature = "online-downloads")]
pub mod downloads;
pub mod error;
pub mod firmware;
pub mod format;
pub mod input;
pub mod json;
pub mod package;
pub mod paths;
pub mod profile;
pub mod synthetic;
pub mod transfer;
pub mod usb;
pub mod verification;

pub use cancel::CancelToken;
pub use error::{Error, Result};

/// Product version shown in the UI and written into reports.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Progress callback used by long-running operations.
pub type Progress<'a> = Option<&'a (dyn Fn(&str) + Sync)>;

pub(crate) fn report(progress: Progress<'_>, text: &str) {
    if let Some(p) = progress {
        p(text);
    }
}
