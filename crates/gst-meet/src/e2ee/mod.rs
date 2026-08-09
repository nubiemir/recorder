//! End-to-end encryption support (work in progress).
//!
//! Jitsi's E2EE derives per-participant media keys over an Olm session, so the
//! recorder needs its own Olm identity to take part in the key exchange.

pub mod olm_adapter;
