use std::sync::Arc;

use libstrophe::Stanza;

use webrtc_sdp::SdpSession;
use webrtc_sdp::attribute_type::{
    SdpAttribute, SdpAttributeCandidateTransport, SdpAttributeCandidateType, SdpAttributeSetup,
};

#[derive(Debug, Clone)]
pub struct IceCandidate {
    pub component: u32,
    pub foundation: String,
    pub protocol: String,
    pub ip: String,
    pub port: u16,
    pub priority: u64,
    pub candidate_type: String, // "host", "srflx", "relay"
    pub rel_addr: Option<String>,
    pub rel_port: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct DtlsFingerprint {
    pub hash_algo: String, // "sha-256"
    pub fingerprint: String,
    pub setup: String, // "actpass" / "active" / "passive"
}

#[derive(Debug, Clone)]
pub struct PayloadType {
    pub id: u8,
    pub name: String,
    pub clockrate: u32,
    pub channels: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct ContentDescription {
    pub media: String, // "audio" / "video"
    pub payloads: Vec<PayloadType>,
    pub ssrcs: Vec<(u32, String)>, // (ssrc, name)
    pub rtcp_mux: bool,
}

#[derive(Debug, Clone)]
pub struct JingleTransport {
    pub ufrag: String,
    pub pwd: String,
    pub fingerprint: DtlsFingerprint,
    pub candidates: Vec<IceCandidate>,
    pub rtcp_mux: bool,
}

#[derive(Debug, Clone)]
pub struct JingleContent {
    pub name: String,
    pub description: ContentDescription,
    pub transport: JingleTransport,
}

#[derive(Debug, Clone)]
pub struct JingleSession {
    pub sid: String,
    pub initiator: String,
    pub to: String,
    pub from: String,
    pub action: String,
    pub contents: Vec<JingleContent>,
    pub bundle: Vec<String>, // content names in BUNDLE group
}

fn parse_candidates(transport: &Stanza) -> Vec<IceCandidate> {
    let mut candidates = Vec::new();
    for c in transport.children() {
        if c.name() == Some("candidate") {
            let candidate = IceCandidate {
                component: c
                    .get_attribute("component")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1),
                foundation: c
                    .get_attribute("foundation")
                    .unwrap_or_default()
                    .to_string(),
                protocol: c.get_attribute("protocol").unwrap_or("udp").to_string(),
                ip: c.get_attribute("ip").unwrap_or_default().to_string(),
                port: c
                    .get_attribute("port")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
                priority: c
                    .get_attribute("priority")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
                candidate_type: c.get_attribute("type").unwrap_or("host").to_string(),
                rel_addr: c.get_attribute("rel-addr").map(|s| s.to_string()),
                rel_port: c.get_attribute("rel-port").and_then(|v| v.parse().ok()),
            };
            candidates.push(candidate);
        }
    }
    candidates
}

fn parse_description(desc: &Stanza) -> ContentDescription {
    let media = desc.get_attribute("media").unwrap_or("audio").to_string();
    let mut payloads = Vec::new();
    let mut ssrcs = Vec::new();
    let mut rtcp_mux = false;

    for c in desc.children() {
        match c.name() {
            Some("payload-type") => {
                let pt = PayloadType {
                    id: c
                        .get_attribute("id")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0),
                    name: c.get_attribute("name").unwrap_or_default().to_string(),
                    clockrate: c
                        .get_attribute("clockrate")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(90000),
                    channels: c.get_attribute("channels").and_then(|v| v.parse().ok()),
                };
                payloads.push(pt);
            }
            Some("source") => {
                // urn:xmpp:jingle:apps:rtp:ssma:0
                if let Some(ssrc_str) = c.get_attribute("ssrc") {
                    if let Ok(ssrc) = ssrc_str.parse::<u32>() {
                        let name = c.get_attribute("name").unwrap_or("unknown").to_string();
                        ssrcs.push((ssrc, name));
                    }
                }
            }
            Some("rtcp-mux") => {
                rtcp_mux = true;
            }
            _ => {}
        }
    }

    ContentDescription {
        media,
        payloads,
        ssrcs,
        rtcp_mux,
    }
}

fn parse_transport(transport: &Stanza) -> Option<JingleTransport> {
    let ufrag = transport.get_attribute("ufrag")?.to_string();
    let pwd = transport.get_attribute("pwd")?.to_string();

    let mut fingerprint_hash = String::new();
    let mut fingerprint_value = String::new();
    let mut fingerprint_setup = String::from("active");
    let mut rtcp_mux = false;

    for c in transport.children() {
        match c.name() {
            Some("fingerprint") => {
                fingerprint_hash = c.get_attribute("hash").unwrap_or("sha-256").to_string();
                fingerprint_setup = c.get_attribute("setup").unwrap_or("actpass").to_string();
                fingerprint_value = c.text().unwrap_or_default().to_string();
            }
            Some("rtcp-mux") => {
                rtcp_mux = true;
            }
            _ => {}
        }
    }

    Some(JingleTransport {
        ufrag,
        pwd,
        fingerprint: DtlsFingerprint {
            hash_algo: fingerprint_hash,
            fingerprint: fingerprint_value,
            setup: fingerprint_setup,
        },
        rtcp_mux,
        candidates: parse_candidates(transport),
    })
}

pub fn parse_session_initiate(jingle: &Stanza, to: &str, from: &str) -> Option<JingleSession> {
    let sid = jingle.get_attribute("sid").unwrap_or_default();
    let initiator = jingle.get_attribute("initiator").unwrap_or_default();

    let mut contents: Vec<JingleContent> = Vec::new();
    let mut bundle = Vec::new();

    for c in jingle.children() {
        match c.name() {
            Some("content") => {
                let name = c.get_attribute("name").unwrap_or("").to_string();
                let mut description = None;
                let mut transport = None;

                for s in c.children() {
                    match s.name() {
                        Some("description") => description = Some(parse_description(&s)),
                        Some("transport") => transport = parse_transport(&s),
                        _ => {}
                    }
                }

                if let (Some(desc), Some(trans)) = (description, transport) {
                    contents.push(JingleContent {
                        name,
                        description: desc,
                        transport: trans,
                    });
                }
            }
            Some("group") => {
                // BUNDLE group
                for s in c.children() {
                    if s.name() == Some("content") {
                        if let Some(n) = s.get_attribute("name") {
                            bundle.push(n.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Some(JingleSession {
        sid: sid.to_string(),
        initiator: initiator.to_string(),
        to: to.to_string(),
        from: from.to_string(),
        action: "session-initiate".to_string(),
        contents,
        bundle,
    })
}

pub fn sdp_jingle_session(
    sdp: SdpSession,
    initiator_jingle_session: Arc<JingleSession>,
) -> JingleSession {
    let contents = initiator_jingle_session
        .contents
        .iter()
        .map(|init_content| {
            // Find the matching SDP media section by mid
            let media = sdp.media.iter().find(|m| {
                m.get_attributes().iter().any(|a| {
                    if let SdpAttribute::Mid(mid) = a {
                        mid == &init_content.name
                    } else {
                        false
                    }
                })
            });

            // --- ICE + DTLS transport from SDP ---
            let mut ufrag = String::new();
            let mut pwd = String::new();
            let mut fp = DtlsFingerprint {
                hash_algo: String::new(),
                fingerprint: String::new(),
                setup: "active".to_string(),
            };
            let mut candidates = Vec::new();

            if let Some(media) = media {
                for attr in media.get_attributes() {
                    match attr {
                        SdpAttribute::IceUfrag(u) => ufrag = u.clone(),
                        SdpAttribute::IcePwd(p) => pwd = p.clone(),

                        SdpAttribute::Fingerprint(f) => {
                            fp.hash_algo = format!("{}", f.hash_algorithm);
                            fp.fingerprint = f
                                .fingerprint
                                .iter()
                                .map(|b| format!("{:02X}", b))
                                .collect::<Vec<_>>()
                                .join(":");
                        }

                        SdpAttribute::Setup(s) => {
                            fp.setup = match s {
                                SdpAttributeSetup::Active => "active",
                                SdpAttributeSetup::Passive => "passive",
                                SdpAttributeSetup::Actpass => "actpass",
                                SdpAttributeSetup::Holdconn => "holdconn",
                            }
                            .to_string();
                        }

                        SdpAttribute::Candidate(c) => {
                            candidates.push(IceCandidate {
                                component: c.component,
                                foundation: c.foundation.clone(),
                                protocol: match c.transport {
                                    SdpAttributeCandidateTransport::Udp => "udp".to_string(),
                                    SdpAttributeCandidateTransport::Tcp => "tcp".to_string(),
                                },
                                ip: c.address.to_string(),
                                port: c.port as u16,
                                priority: c.priority,
                                candidate_type: match c.c_type {
                                    SdpAttributeCandidateType::Host => "host",
                                    SdpAttributeCandidateType::Srflx => "srflx",
                                    SdpAttributeCandidateType::Prflx => "prflx",
                                    SdpAttributeCandidateType::Relay => "relay",
                                }
                                .to_string(),
                                rel_addr: c.raddr.as_ref().map(|a| a.to_string()),
                                rel_port: c.rport.map(|p| p as u16),
                            });
                        }

                        _ => {}
                    }
                }
            }

            JingleContent {
                name: init_content.name.clone(),
                description: ContentDescription {
                    media: init_content.description.media.clone(),
                    // Take these from initiator — correct casing, correct extensions
                    payloads: init_content.description.payloads.clone(),
                    rtcp_mux: init_content.description.rtcp_mux,
                    ssrcs: vec![], // recvonly — no outgoing streams
                },
                transport: JingleTransport {
                    ufrag,
                    pwd,
                    fingerprint: fp,
                    rtcp_mux: init_content.transport.rtcp_mux,
                    candidates,
                },
            }
        })
        .collect();

    JingleSession {
        sid: initiator_jingle_session.sid.clone(),
        initiator: initiator_jingle_session.to.to_string(),
        to: initiator_jingle_session.from.to_string(),
        from: initiator_jingle_session.to.to_string(),
        action: "session-accept".to_string(),
        contents,
        bundle: initiator_jingle_session.bundle.clone(),
    }
}

pub fn parse_ice_candidate_string(candidate_str: &str) -> Option<IceCandidate> {
    // GStreamer gives: "candidate:1 1 UDP 2015363327 192.168.1.135 61078 typ host"
    // strip "candidate:" prefix if present
    let s = candidate_str.trim_start_matches("candidate:").trim();

    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 8 {
        return None;
    }

    // candidate:<foundation> <component> <protocol> <priority> <ip> <port> typ <type> [raddr <addr> rport <port>]
    let foundation = parts[0].to_string();
    let component: u32 = parts[1].parse().ok()?;
    let protocol = parts[2].to_lowercase();
    let priority: u64 = parts[3].parse().ok()?;
    let ip = parts[4].to_string();
    let port: u16 = parts[5].parse().ok()?;
    // parts[6] == "typ"
    let candidate_type = parts[7].to_string();

    let mut rel_addr = None;
    let mut rel_port = None;

    // parse optional raddr/rport
    let mut i = 8;
    while i < parts.len() {
        match parts[i] {
            "raddr" => {
                rel_addr = parts.get(i + 1).map(|s| s.to_string());
                i += 2;
            }
            "rport" => {
                rel_port = parts.get(i + 1).and_then(|s| s.parse().ok());
                i += 2;
            }
            _ => i += 1,
        }
    }

    Some(IceCandidate {
        foundation,
        component,
        protocol,
        priority,
        ip,
        port,
        candidate_type,
        rel_addr,
        rel_port,
    })
}
