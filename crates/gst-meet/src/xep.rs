//! XMPP protocol namespaces, kept in one place so stanza builders never spell
//! a URN by hand.

use std::fmt;

/// A namespace URN used in the Jingle stanzas this client exchanges with the
/// focus and the bridge. [`Display`](fmt::Display) yields the URN itself, so
/// these go straight into `xmlns` attributes.
pub enum XEP {
    /// XEP-0338 grouping, i.e. BUNDLE.
    BundleMedia,
    /// XEP-0320 DTLS-SRTP key exchange.
    DtlsSrtp,
    /// XEP-0176 ICE-UDP transport.
    IceUdpTransport,
    /// XEP-0166 Jingle itself.
    Jingle,
    /// XEP-0327 Rayo, used by Jitsi for call control.
    Rayo,
    /// XEP-0167 audio content type.
    RtpAudio,
    /// XEP-0293 RTCP feedback negotiation.
    RtpFeedback,
    /// XEP-0294 RTP header extensions.
    RtpHeaderExtensions,
    /// XEP-0167 RTP sessions.
    RtpMedia,
    /// XEP-0167 video content type.
    RtpVideo,
    /// SCTP data channel transport.
    ///
    /// NOTE: currently resolves to the same URN as [`XEP::RtpMedia`]
    /// (`urn:xmpp:jingle:apps:rtp:1`), which is almost certainly wrong —
    /// the SCTP transport is `urn:xmpp:jingle:transports:dtls-sctp:1`.
    SctpDataChannel,
    /// XEP-0339 source-specific media attributes (`ssrc` / `ssrc-group`).
    SourceAttributes,
}

impl fmt::Display for XEP {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BundleMedia => write!(f, "urn:xmpp:jingle:apps:grouping:0"),
            Self::DtlsSrtp => write!(f, "urn:xmpp:jingle:apps:dtls:0"),
            Self::IceUdpTransport => write!(f, "urn:xmpp:jingle:transports:ice-udp:1"),
            Self::Jingle => write!(f, "urn:xmpp:jingle:1"),
            Self::Rayo => write!(f, "urn:xmpp:rayo:client:1"),
            Self::RtpAudio => write!(f, "urn:xmpp:jingle:apps:rtp:audio"),
            Self::RtpFeedback => write!(f, "urn:xmpp:jingle:apps:rtp:rtcp-fb:0"),
            Self::RtpHeaderExtensions => write!(f, "urn:xmpp:jingle:apps:rtp:rtp-hdrext:0"),
            Self::RtpMedia => write!(f, "urn:xmpp:jingle:apps:rtp:1"),
            Self::RtpVideo => write!(f, "urn:xmpp:jingle:apps:rtp:video"),
            Self::SctpDataChannel => write!(f, "urn:xmpp:jingle:apps:rtp:1"),
            Self::SourceAttributes => write!(f, "urn:xmpp:jingle:apps:rtp:ssma:0"),
        }
    }
}
