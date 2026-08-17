//! End-to-end encryption support (work in progress).
//!
//! Jitsi's E2EE derives per-participant media keys over an Olm session, so the
//! recorder needs its own Olm identity to take part in the key exchange.

use crate::e2ee::olm_adapter::OlmAdapter;

pub mod olm_adapter;

#[derive(Debug)]
#[allow(unused)]
pub struct E2EE {
    olm_adapter: OlmAdapter,
}

impl E2EE {
    pub fn new() -> Self {
        Self {
            olm_adapter: OlmAdapter::new(),
        }
    }
}
