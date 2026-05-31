use std::fmt;

pub enum XEP {
    BundleMedia,
    DtlsSrtp,
    IceUdpTransport,
    Jingle,
    Rayo,
    RtpAudio,
    RtpFeedback,
    RtpHeaderExtensions,
    RtpMedia,
    RtpVideo,
    SctpDataChannel,
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
