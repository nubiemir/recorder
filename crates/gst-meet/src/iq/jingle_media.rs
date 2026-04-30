use libstrophe::Stanza;

use crate::{
    get_attribute,
    util::{exists, find_all, find_first},
};

#[derive(Debug, Default)]
pub(crate) struct JingleMedia {
    pub media: String,
    pub port: String,
    pub proto: String,
    pub fmt: Vec<String>,
}

impl JingleMedia {
    pub(crate) fn new() -> Self {
        JingleMedia::default()
    }

    pub(crate) fn parse_sdp_media(&mut self, stanza: &Stanza) -> String {
        let mut sdp = String::new();

        let desc = find_first(Some(&stanza), "description");
        let transport = find_first(Some(&stanza), "transport");
        let sctp = find_first(transport.as_ref(), "sctpmap");

        let content_stanza = get_attribute!(stanza, [name, senders]);
        self.set_media(&content_stanza.name);
        self.set_port(&content_stanza.senders);

        let find_exists = exists(transport.as_ref(), "fingerprint");
        self.set_proto(find_exists, sctp.as_ref());

        if let Some(sctp) = sctp.as_ref() {
            sdp.push_str(
                format!(
                    "m=application {} UDP/DTLS/SCTP webrtc-datachannel\r\n",
                    self.port
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
                .map(|payload_type| get_attribute!(payload_type, [id]).id)
                .filter(|id| !id.is_empty())
                .collect();

            self.set_fmt(fmt);
            sdp.push_str(
                format!(
                    "m={} {} {} {}",
                    self.media,
                    self.port,
                    self.proto,
                    self.fmt.join(" ")
                )
                .as_str(),
            );

            sdp.push_str("\r\n");
        }

        sdp.push_str("c=IN IP4 0.0.0.0\r\n");

        if let None = sctp {
            sdp.push_str("a=rtcp:1 IN IP4 0.0.0.0\r\n");
        }
        if let Some(trans) = transport.as_ref() {
            let transport_stanza = get_attribute!(trans, [ufrag, pwd]);
            sdp.push_str(format!("a=ice-ufrag:{}\r\n", transport_stanza.ufrag).as_str());
            sdp.push_str(format!("a=ice-pwd:{}\r\n", transport_stanza.pwd).as_str());

            find_all(transport.as_ref(), "fingerprint")
                .iter()
                .for_each(|fg| {
                    let fg_stanza = get_attribute!(fg, [hash, setup]);
                    let text = fg.text().unwrap_or_default();
                    sdp.push_str(format!("a=fingerprint:{} {}\r\n", fg_stanza.hash, text).as_str());

                    if !fg_stanza.setup.is_empty() {
                        sdp.push_str(format!("a=setup:{}\r\n", fg_stanza.setup).as_str());
                    }
                });

            find_all(transport.as_ref(), "candidate")
                .iter()
                .for_each(|cand| {
                    self.parse_candidate(cand);
                });
        }

        self.set_senders(stanza, &mut sdp);
        sdp.push_str(format!("a=mid:{}\r\n", content_stanza.name).as_str());

        if exists(desc.as_ref(), "rtcp-mux") {
            sdp.push_str("a=rtcp-mux\r\n");
        }

        sdp
    }

    fn parse_candidate(&mut self, stanza: &Stanza) -> String {
        let mut sdp = String::new();
        sdp.push_str("a=candidate:");

        let candidate_stanza = get_attribute!(stanza, {
            foundation => "foundation",
            component => "component",
            protocol => "protocol",
            priority => "priority",
            ip => "ip",
            port => "port",
            kind => "type",
            rel_addr => "rel-addr",
            rel_port => "rel-port",
            tcp_type => "tcptype",
            generation => "generation"
        });

        sdp.push_str(
            format!(
                "{} {} {} {} {} {} typ {} ",
                candidate_stanza.foundation,
                candidate_stanza.component,
                candidate_stanza.protocol,
                candidate_stanza.priority,
                candidate_stanza.ip,
                candidate_stanza.port,
                candidate_stanza.kind
            )
            .as_str(),
        );

        match candidate_stanza.kind.as_str() {
            "srflx" | "prflx" | "relay" => {
                if !candidate_stanza.rel_port.is_empty() && !candidate_stanza.rel_addr.is_empty() {
                    sdp.push_str(
                        format!(
                            "raddr {} rport {} ",
                            candidate_stanza.rel_addr, candidate_stanza.rel_port
                        )
                        .as_str(),
                    );
                }
            }
            _ => {}
        }

        if candidate_stanza.protocol.to_lowercase() == "tcp" {
            sdp.push_str(format!("tcptype {} ", candidate_stanza.tcp_type,).as_str());
        }

        sdp.push_str(format!("generation {}", candidate_stanza.generation).as_str());

        sdp.push_str("\r\n");

        sdp
    }

    fn set_media(&mut self, attr: &str) {
        self.media = attr.to_string();
    }

    fn set_port(&mut self, attr: &str) {
        if !attr.is_empty() && attr == "rejected" {
            self.port = '0'.to_string();
        } else {
            self.port = '9'.to_string();
        }
    }

    fn set_proto(&mut self, fingerprint_exist: bool, sctp: Option<&Stanza>) {
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

    fn set_senders(&self, stanza: &Stanza, sdp: &mut String) {
        let content_stanza = get_attribute!(stanza, [senders]);
        match content_stanza.senders.as_str() {
            "initiator" => sdp.push_str("a=recvonly\r\n"),
            "responder" => sdp.push_str("a=sendonly\r\n"),
            "none" => sdp.push_str("a=inactive\r\n"),
            "both" => sdp.push_str("a=sendrecv\r\n"),
            _ => {}
        }
    }

    fn set_fmt(&mut self, fmt: Vec<String>) {
        self.fmt = fmt;
    }
}
