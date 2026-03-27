use std::sync::Arc;

use chrono::Utc;
use libstrophe::Stanza;
use log::info;

use crate::{
    jingle::{IceCandidate, JingleSession, PayloadType},
    sdp_util::{exists, find_all, find_first},
};

pub fn parse_jingle_sdp(session: Arc<JingleSession>) -> String {
    let mut sdp = String::from("");
    sdp.push_str("v=0\r\n");
    sdp.push_str("o=- 0 0 IN IP4 0.0.0.0\r\n");
    sdp.push_str("s=-\r\n");
    sdp.push_str("t=0 0\r\n");

    // Bundle group — JVB always bundles audio+video on same port
    let bundle_mids = session.bundle.join(" ");
    sdp.push_str(&format!("a=group:BUNDLE {}\r\n", bundle_mids));

    for content in &session.contents {
        let transport = &content.transport;
        let desc = &content.description;

        // m= line
        let port = transport.candidates.first().map(|c| c.port).unwrap_or(9);
        sdp.push_str(&format!(
            "m={} {} UDP/TLS/RTP/SAVPF {}\r\n",
            desc.media,
            port,
            desc.payloads
                .iter()
                .map(|p| p.id.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        ));

        sdp.push_str("c=IN IP4 0.0.0.0\r\n");
        sdp.push_str("a=rtcp-mux\r\n");

        // ICE credentials
        sdp.push_str(&format!("a=ice-ufrag:{}\r\n", transport.ufrag));
        sdp.push_str(&format!("a=ice-pwd:{}\r\n", transport.pwd));

        // DTLS fingerprint
        // JVB says "actpass" — we must pick either active or passive
        // as answerer we pick "active" (we initiate DTLS)
        sdp.push_str(&format!(
            "a=fingerprint:{} {}\r\n",
            transport.fingerprint.hash_algo, transport.fingerprint.fingerprint
        ));
        sdp.push_str("a=setup:active\r\n");

        // ICE candidates
        for c in &transport.candidates {
            let mut cand = format!(
                "a=candidate:{} {} {} {} {} {} typ {}",
                c.foundation, c.component, c.protocol, c.priority, c.ip, c.port, c.candidate_type
            );
            if let (Some(ra), Some(rp)) = (&c.rel_addr, c.rel_port) {
                cand.push_str(&format!(" raddr {} rport {}", ra, rp));
            }
            cand.push_str("\r\n");
            sdp.push_str(&cand);
        }

        // Payload types
        for payload in &desc.payloads {
            sdp.push_str(&format!(
                "a=rtpmap:{} {}/{}",
                payload.id, payload.name, payload.clockrate
            ));
            if let Some(ch) = payload.channels {
                sdp.push_str(&format!("/{}", ch));
            }
            sdp.push_str("\r\n");

            // Opus needs these fmtp
            if payload.name == "opus" {
                sdp.push_str(&format!(
                    "a=fmtp:{} minptime=10;useinbandfec=1\r\n",
                    payload.id
                ));
            }
        }

        // SSRCs from JVB (what it will send us)
        for (ssrc, label) in &desc.ssrcs {
            sdp.push_str(&format!("a=ssrc:{} cname:{}\r\n", ssrc, label));
        }

        // Direction — JVB is sending to us, we receive
        sdp.push_str("a=sendonly\r\n");

        // mid must match bundle
        sdp.push_str(&format!("a=mid:{}\r\n", content.name));
    }

    sdp
}

pub fn parse_sdp_jingle(to: &str, from: &str, session: &JingleSession) -> Stanza {
    let mut iq = Stanza::new_iq(Some("set"), None);
    iq.set_to(to).unwrap();
    iq.set_from(from).unwrap();

    let mut jingle = Stanza::new();
    jingle.set_name("jingle").unwrap();
    jingle.set_ns("urn:xmpp:jingle:1").unwrap();
    jingle.set_attribute("action", "session-accept").unwrap();
    jingle.set_attribute("sid", &session.sid).unwrap();
    jingle
        .set_attribute("responder", &session.initiator)
        .unwrap();

    // BUNDLE group
    if !session.bundle.is_empty() {
        let mut group = Stanza::new();
        group.set_name("group").unwrap();
        group.set_ns("urn:xmpp:jingle:apps:grouping:0").unwrap();
        group.set_attribute("semantics", "BUNDLE").unwrap();
        for name in &session.bundle {
            let mut c = Stanza::new();
            c.set_name("content").unwrap();
            c.set_attribute("name", name).unwrap();
            group.add_child(c).unwrap();
        }
        jingle.add_child(group).unwrap();
    }

    for content in &session.contents {
        let mut content_stanza = Stanza::new();
        content_stanza.set_name("content").unwrap();
        content_stanza.set_attribute("name", &content.name).unwrap();
        content_stanza
            .set_attribute("creator", "initiator")
            .unwrap();
        content_stanza.set_attribute("senders", "both").unwrap();

        // <description>
        let mut desc = Stanza::new();
        desc.set_name("description").unwrap();
        desc.set_ns("urn:xmpp:jingle:apps:rtp:1").unwrap();
        desc.set_attribute("media", &content.description.media)
            .unwrap();

        // payload types
        for pt in &content.description.payloads {
            let mut payload = Stanza::new();
            payload.set_name("payload-type").unwrap();
            payload.set_attribute("id", &pt.id.to_string()).unwrap();
            payload.set_attribute("name", &pt.name).unwrap();
            payload
                .set_attribute("clockrate", &pt.clockrate.to_string())
                .unwrap();
            if let Some(ch) = pt.channels {
                payload.set_attribute("channels", &ch.to_string()).unwrap();
            }
            desc.add_child(payload).unwrap();
        }

        // rtcp-mux inside description
        if content.description.rtcp_mux {
            let mut rtcp_mux = Stanza::new();
            rtcp_mux.set_name("rtcp-mux").unwrap();
            desc.add_child(rtcp_mux).unwrap();
        }

        // <transport>
        let trans = &content.transport;
        let mut transport = Stanza::new();
        transport.set_name("transport").unwrap();
        transport
            .set_ns("urn:xmpp:jingle:transports:ice-udp:1")
            .unwrap();
        transport.set_attribute("ufrag", &trans.ufrag).unwrap();
        transport.set_attribute("pwd", &trans.pwd).unwrap();

        // fingerprint
        let mut fp = Stanza::new();
        fp.set_name("fingerprint").unwrap();
        fp.set_ns("urn:xmpp:jingle:apps:dtls:0").unwrap();
        fp.set_attribute("hash", &trans.fingerprint.hash_algo)
            .unwrap();
        fp.set_attribute("setup", &trans.fingerprint.setup).unwrap();
        let mut fp_text = Stanza::new();
        fp_text.set_text(&trans.fingerprint.fingerprint).unwrap();
        fp.add_child(fp_text).unwrap();
        transport.add_child(fp).unwrap();

        // rtcp-mux inside transport
        if content.transport.rtcp_mux {
            let mut rtcp_mux = Stanza::new();
            rtcp_mux.set_name("rtcp-mux").unwrap();
            transport.add_child(rtcp_mux).unwrap();
        }

        // candidates
        for c in &trans.candidates {
            let mut cand = Stanza::new();
            cand.set_name("candidate").unwrap();
            cand.set_attribute("component", &c.component.to_string())
                .unwrap();
            cand.set_attribute("foundation", &c.foundation).unwrap();
            cand.set_attribute("protocol", &c.protocol).unwrap();
            cand.set_attribute("priority", &c.priority.to_string())
                .unwrap();
            cand.set_attribute("ip", &c.ip).unwrap();
            cand.set_attribute("port", &c.port.to_string()).unwrap();
            cand.set_attribute("type", &c.candidate_type).unwrap();
            cand.set_attribute("generation", "0").unwrap();
            if let (Some(addr), Some(port)) = (&c.rel_addr, c.rel_port) {
                cand.set_attribute("rel-addr", addr).unwrap();
                cand.set_attribute("rel-port", &port.to_string()).unwrap();
            }
            transport.add_child(cand).unwrap();
        }

        content_stanza.add_child(desc).unwrap();
        content_stanza.add_child(transport).unwrap();
        jingle.add_child(content_stanza).unwrap();
    }

    iq.add_child(jingle).unwrap();

    info!("IQ received: {:?}", iq.to_string());
    iq
}

pub fn parse_transport_jingle(
    sid: &str,
    content_name: &str,
    ufrag: &str,
    pwd: &str,
    candidate: &IceCandidate,
) -> Stanza {
    // <iq type="set">
    let mut iq = Stanza::new_iq(Some("set"), None);

    // <jingle>
    let mut jingle = Stanza::new();
    jingle.set_name("jingle").unwrap();
    jingle.set_ns("urn:xmpp:jingle:1").unwrap();
    jingle.set_attribute("action", "transport-info").unwrap();
    jingle.set_attribute("sid", sid).unwrap();

    // <content>
    let mut content = Stanza::new();
    content.set_name("content").unwrap();
    content.set_attribute("name", content_name).unwrap();
    content.set_attribute("creator", "initiator").unwrap();

    // <transport>
    let mut transport = Stanza::new();
    transport.set_name("transport").unwrap();
    transport
        .set_ns("urn:xmpp:jingle:transports:ice-udp:1")
        .unwrap();
    transport.set_attribute("ufrag", ufrag).unwrap();
    transport.set_attribute("pwd", pwd).unwrap();

    // <candidate>
    let mut cand = Stanza::new();
    cand.set_name("candidate").unwrap();
    cand.set_attribute("component", &candidate.component.to_string())
        .unwrap();
    cand.set_attribute("foundation", &candidate.foundation)
        .unwrap();
    cand.set_attribute("protocol", &candidate.protocol).unwrap();
    cand.set_attribute("priority", &candidate.priority.to_string())
        .unwrap();
    cand.set_attribute("ip", &candidate.ip).unwrap();
    cand.set_attribute("port", &candidate.port.to_string())
        .unwrap();
    cand.set_attribute("type", &candidate.candidate_type)
        .unwrap();
    cand.set_attribute("generation", "0").unwrap();
    cand.set_attribute("network", "0").unwrap();
    if let (Some(addr), Some(port)) = (&candidate.rel_addr, candidate.rel_port) {
        cand.set_attribute("rel-addr", addr).unwrap();
        cand.set_attribute("rel-port", &port.to_string()).unwrap();
    }

    transport.add_child(cand).unwrap();
    content.add_child(transport).unwrap();
    jingle.add_child(content).unwrap();
    iq.add_child(jingle).unwrap();
    iq
}

pub fn build_transport_info(
    sid: &str,
    content_name: &str,
    ufrag: &str,
    pwd: &str,
    candidate: &IceCandidate,
) -> Stanza {
    let mut iq = Stanza::new();
    iq.set_name("iq").unwrap();
    iq.set_attribute("type", "set").unwrap();

    let mut jingle = Stanza::new();
    jingle.set_name("jingle").unwrap();
    jingle.set_attribute("xmlns", "urn:xmpp:jingle:1").unwrap();
    jingle.set_attribute("action", "transport-info").unwrap();
    jingle.set_attribute("sid", sid).unwrap();

    let mut content = Stanza::new();
    content.set_name("content").unwrap();
    content.set_attribute("name", content_name).unwrap();
    content.set_attribute("creator", "initiator").unwrap();

    let mut transport = Stanza::new();
    transport.set_name("transport").unwrap();
    transport
        .set_attribute("xmlns", "urn:xmpp:jingle:transports:ice-udp:1")
        .unwrap();
    transport.set_attribute("ufrag", ufrag).unwrap();
    transport.set_attribute("pwd", pwd).unwrap();

    let mut cand = Stanza::new();
    cand.set_name("candidate").unwrap();
    cand.set_attribute("component", &candidate.component.to_string())
        .unwrap();
    cand.set_attribute("foundation", &candidate.foundation)
        .unwrap();
    cand.set_attribute("protocol", &candidate.protocol).unwrap();
    cand.set_attribute("priority", &candidate.priority.to_string())
        .unwrap();
    cand.set_attribute("ip", &candidate.ip).unwrap();
    cand.set_attribute("port", &candidate.port.to_string())
        .unwrap();
    cand.set_attribute("type", &candidate.candidate_type)
        .unwrap();
    cand.set_attribute("generation", "0").unwrap();
    cand.set_attribute("network", "0").unwrap();
    if let (Some(addr), Some(port)) = (&candidate.rel_addr, candidate.rel_port) {
        cand.set_attribute("rel-addr", addr).unwrap();
        cand.set_attribute("rel-port", &port.to_string()).unwrap();
    }

    transport.add_child(cand).unwrap();
    content.add_child(transport).unwrap();
    jingle.add_child(content).unwrap();
    iq.add_child(jingle).unwrap();
    iq
}

#[derive(Debug, Default)]
pub struct JingleMedia {
    pub media: Option<String>,
    pub port: String,
    pub proto: String,
    pub fmt: Vec<String>,
}

impl JingleMedia {
    pub fn new() -> Self {
        JingleMedia::default()
    }

    pub fn set_media(&mut self, attr: Option<&str>) {
        if let Some(media) = attr {
            self.media = Some(media.to_string());
        } else {
            self.media = None;
        }
    }

    pub fn set_port(&mut self, attr: Option<&str>) {
        if let Some(senders) = attr
            && senders == "rejected"
        {
            self.port = '0'.to_string();
        } else {
            self.port = '9'.to_string();
        }
    }

    pub fn set_proto(&mut self, fingerprint_exist: bool, sctp: Option<&Stanza>) {
        if fingerprint_exist {
            match sctp {
                Some(_) => {
                    self.proto = "UDP/DTLS/SCTP".to_string();
                }
                None => {
                    self.proto = "UDP/TLS/RTP/SAVPF".to_string();
                }
            }
        } else {
            self.proto = "UDP/TLS/RTP/SAVPF".to_string();
        }
    }

    pub fn set_fmt(&mut self, fmt: Vec<String>) {
        self.fmt = fmt;
    }
}

pub fn jingle_2_media<'a>(content: &Stanza, sdp: &'a mut String) -> &'a String {
    let desc = find_first(Some(&content), "description");
    let transport = find_first(Some(&content), "transport");
    let sctp = find_first(transport.as_ref(), "sctpmap");
    let mid = content.get_attribute("name");
    let mut media = JingleMedia::new();
    media.set_media(content.get_attribute("media"));
    media.set_port(content.get_attribute("senders"));
    let exists = exists(transport.as_ref(), "fingerprint");
    media.set_proto(exists, sctp.as_ref());

    if let Some(sctp) = sctp.as_ref() {
        sdp.push_str(
            format!(
                "m=application {} UDP/DTLS/SCTP webrtc-datachannel\r\n",
                media.port
            )
            .as_str(),
        );
        if let Some(num) = sctp.get_attribute("number") {
            sdp.push_str(format!("a=sctp-port:{}\r\n", num).as_str());
            sdp.push_str("a=max-message-size:262144\r\n");
        }
    } else {
        let fmt: Vec<String> = find_all(desc.as_ref(), "payload-type")
            .iter()
            .map(|payload_type| {
                if let Some(id) = payload_type.get_attribute("id") {
                    return id.to_string();
                }
                String::new()
            })
            .filter(|id| !id.is_empty())
            .collect();
        media.set_fmt(fmt);
        sdp.push_str(
            format!(
                "m={} {} {} {}",
                media.media.unwrap_or_default(),
                media.port,
                media.proto,
                media.fmt.join(" ")
            )
            .as_str(),
        );
    }

    sdp.push_str("c=IN IP4 0.0.0.0\r\n");

    if let None = sctp {
        sdp.push_str("a=rtcp:1 IN IP4 0.0.0.0\r\n");
    }
    if let Some(trans) = transport.as_ref() {
        let ufrag = trans.get_attribute("ufrag");
        let pwd = trans.get_attribute("pwd");

        if let Some(frag) = ufrag {
            sdp.push_str(format!("a=ice-ufrag:{}\r\n", frag).as_str());
        }

        if let Some(pwd) = pwd {
            sdp.push_str(format!("a=ice-pwd:{}\r\n", pwd).as_str());
        }

        find_all(transport.as_ref(), "fingerprint")
            .iter()
            .for_each(|fg| {
                let hash = fg.get_attribute("hash").unwrap_or_default();
                let text = fg.text().unwrap_or_default();
                sdp.push_str(format!("a=fingerprint:{} {}\r\n", hash, text).as_str());

                if let Some(setup) = fg.get_attribute("setup") {
                    sdp.push_str(format!("a=setup:{}\r\n", setup).as_str());
                }
            });

        find_all(transport.as_ref(), "candidate")
            .iter()
            .for_each(|cand| {
                let protocol = cand
                    .get_attribute("protocol")
                    .unwrap_or_default()
                    .to_lowercase();
            });

        // findAll(transport, ':scope>candidate').forEach(candidate => {
        //     let protocol = getAttribute(candidate, 'protocol');
        //
        //     protocol = typeof protocol === 'string' ? protocol.toLowerCase() : '';
        //
        //     if ((this.removeTcpCandidates && (protocol === 'tcp' || protocol === 'ssltcp'))
        //         || (this.removeUdpCandidates && protocol === 'udp')) {
        //         return;
        //     } else if (this.failICE) {
        //         candidate.setAttribute('ip', '1.1.1.1');
        //     }
        //
        //     sdp += SDPUtil.candidateFromJingle(candidate);
        // });
    }

    sdp
}

pub fn from_jingle(jingle: Option<Stanza>) -> String {
    let mut sdp = String::new();

    let session_id = Utc::now().timestamp_millis();

    sdp.push_str("v=0\r\n");
    sdp.push_str("o=- ");
    sdp.push_str(session_id.to_string().as_str());
    sdp.push_str(" 2 IN IP4 0.0.0.0\r\n");
    sdp.push_str("s=-\r\n");
    sdp.push_str("t=0 0\r\n");

    let fingerprints = find_all(jingle.as_ref(), "content>transport>fingerprint");
    let has_cryptex = fingerprints
        .iter()
        .any(|x| x.get_attribute("cryptex") == Some("true"));

    if has_cryptex {
        sdp.push_str("a=cryptex\r\n");
    }

    let content = find_all(jingle.as_ref(), "content")
        .iter()
        .for_each(|content| {
            let media = jingle_2_media(content, &mut sdp);
        });

    sdp
}
