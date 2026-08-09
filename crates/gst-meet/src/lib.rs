//! A headless Jitsi Meet participant that records a conference.
//!
//! The recorder joins a room as an ordinary XMPP client, negotiates a single
//! WebRTC connection with the JVB, and writes one file per incoming stream
//! alongside a timeline of what happened in the meeting.
//!
//! # Flow
//!
//! 1. [`xmpp`] connects to the XMPP server and dispatches incoming stanzas.
//! 2. [`presence`] turns MUC presence into join / leave / mute events, which
//!    [`room_manager`] routes to the right [`room::Room`].
//! 3. [`iq`] handles Jingle: it answers `session-initiate` (via [`sdp`]) and
//!    records which SSRC belongs to which participant source.
//! 4. [`room::Room`] owns the GStreamer pipeline. Each incoming RTP pad
//!    becomes a [`participant::branch::Branch`] that depayloads, muxes, and
//!    writes one file.
//! 5. [`timeline`] records joins, mutes and speaker changes in parallel, so a
//!    later render pass can lay the recordings out over time.
//!
//! Output for a meeting lands under `recordings/<room>/`, with per-participant
//! media in `recordings/<room>/<endpoint>/`.

pub mod avatar;
pub mod config;
pub mod e2ee;
pub mod iq;
pub mod macros;
pub mod participant;
pub mod presence;
pub mod room;
pub mod room_manager;
pub mod sdp;
pub mod timeline;
pub mod util;
pub mod xep;
pub mod xmpp;
