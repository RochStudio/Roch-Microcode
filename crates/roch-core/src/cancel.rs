use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::{Error, Result};

/// Cooperative cancellation shared between the UI and worker threads.
#[derive(Clone, Default, Debug)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    /// A token that is never cancelled (`CancellationToken.None`).
    pub fn none() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    /// `ThrowIfCancellationRequested`.
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}
