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
        if let Some(transport) = transport.as_ref() {
            let res = self.parse_transport(transport);
            sdp.push_str(&res);
        }

        self.set_senders(stanza, &mut sdp);
        sdp.push_str(format!("a=mid:{}\r\n", content_stanza.name).as_str());

        if exists(desc.as_ref(), "rtcp-mux") {
            sdp.push_str("a=rtcp-mux\r\n");
        }

        if let Some(desc) = desc {
            sdp.push_str(&self.parse_description(&desc, &content_stanza.name));
        }

        sdp
    }

    fn parse_transport(&mut self, stanza: &Stanza) -> String {
        let mut sdp = String::new();
        let transport_stanza = get_attribute!(stanza, [ufrag, pwd]);
        sdp.push_str(format!("a=ice-ufrag:{}\r\n", transport_stanza.ufrag).as_str());
        sdp.push_str(format!("a=ice-pwd:{}\r\n", transport_stanza.pwd).as_str());

        find_all(Some(stanza), "fingerprint").iter().for_each(|fg| {
            let fg_stanza = get_attribute!(fg, [hash, setup]);
            let text = fg.text().unwrap_or_default();
            sdp.push_str(format!("a=fingerprint:{} {}\r\n", fg_stanza.hash, text).as_str());

            if !fg_stanza.setup.is_empty() {
                sdp.push_str(format!("a=setup:{}\r\n", fg_stanza.setup).as_str());
            }
        });

        find_all(Some(stanza), "candidate").iter().for_each(|cand| {
            let res = self.parse_candidate(cand);
            sdp.push_str(&res);
        });

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

    fn parse_description(&mut self, stanza: &Stanza, mid: &str) -> String {
        let mut sdp = String::new();
        find_all(Some(&stanza), "payload-type")
            .iter()
            .for_each(|payload_type| {
                sdp.push_str(&self.parse_rtp_map(payload_type));
                let parameters = find_all(Some(payload_type), "parameter");
                let id = get_attribute!(payload_type, [id]).id;
                if parameters.len() > 0 {
                    sdp.push_str(format!("a=fmtp:{} ", id).as_str());
                    let params = parameters
                        .iter()
                        .map(|param| {
                            let param_stanza = get_attribute!(param, [value, name]);

                            if !param_stanza.name.is_empty() {
                                format!("{}={}", param_stanza.name, param_stanza.value)
                            } else {
                                param_stanza.value
                            }
                        })
                        .collect::<Vec<String>>()
                        .join(";");

                    sdp.push_str(params.as_str());
                    sdp.push_str("\r\n");
                }

                sdp.push_str(&self.parse_rtcp_fb(payload_type, &id));
            });

        sdp.push_str(&self.parse_rtcp_fb(stanza, "*"));

        find_all(Some(stanza), "rtp-hdrext")
            .iter()
            .for_each(|hdr_ext| {
                let hdrext_stanza = get_attribute!(hdr_ext, [id, uri]);
                sdp.push_str(
                    format!("a=extmap:{} {}\r\n", hdrext_stanza.id, hdrext_stanza.uri).as_str(),
                );
            });

        if exists(Some(stanza), "extmap-allow-mixed") {
            sdp.push_str("a=extmap-allow-mixed\r\n");
        }

        find_all(Some(stanza), "ssrc-group")
            .iter()
            .for_each(|ssrc_group| {
                let semantics = get_attribute!(ssrc_group, [semantics]).semantics;
                let ssrcs: Vec<String> = ssrc_group
                    .children()
                    .filter_map(|child| {
                        (child.name() == Some("source"))
                            .then(|| get_attribute!(child, [ssrc]).ssrc)
                            .map(String::from)
                    })
                    .collect();

                if ssrcs.len() > 0 {
                    sdp.push_str(
                        format!("a=ssrc-group:{} {}\r\n", semantics, ssrcs.join(" ")).as_str(),
                    );
                }
            });

        let mut user_sources = String::new();
        let mut non_user_sources = String::new();

        find_all(Some(stanza), "source").iter().for_each(|source| {
            let ssrc = get_attribute!(source, [ssrc]).ssrc;
            let mut is_user_source = true;
            let mut source_str = String::new();

            find_all(Some(source), "parameter")
                .iter()
                .for_each(|parameter| {
                    let mut param_stanza = get_attribute!(parameter, [name, value]);

                    param_stanza.value = param_stanza
                        .value
                        .chars()
                        .filter(|c| !matches!(c, '\\' | '/' | '{' | '}' | ',' | '+'))
                        .collect();

                    source_str.push_str(format!("a=ssrc:{} {}", ssrc, param_stanza.name).as_str());

                    if param_stanza.name == "msid" {
                        param_stanza.value = self.adjust_msid_semantic(&param_stanza.value, mid);
                    }

                    if param_stanza.value.len() > 0 {
                        source_str.push_str(format!(":{}", param_stanza.value).as_str());
                    }
                    source_str.push_str("\r\n");

                    if param_stanza.value.contains("mixedmslabel") {
                        is_user_source = false;
                    }
                });

            if is_user_source {
                user_sources.push_str(source_str.as_str());
            } else {
                non_user_sources.push_str(source_str.as_str());
            }
        });

        sdp.push_str(non_user_sources.as_str());
        sdp.push_str(user_sources.as_str());
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

    fn parse_rtp_map(&self, stanza: &Stanza) -> String {
        let mut sdp = String::new();

        let rtpmap_stanza = get_attribute!(stanza, [id, name, clockrate, channels]);

        sdp.push_str(
            format!(
                "a=rtpmap:{} {}/{}",
                rtpmap_stanza.id, rtpmap_stanza.name, rtpmap_stanza.clockrate
            )
            .as_str(),
        );

        if !rtpmap_stanza.channels.is_empty() && rtpmap_stanza.channels != "1" {
            sdp.push_str("/");
            sdp.push_str(&rtpmap_stanza.channels);
        }

        sdp.push_str("\r\n");

        sdp
    }

    fn parse_rtcp_fb(&self, payload: &Stanza, attr: &str) -> String {
        let mut sdp = String::new();
        let fb_ele_trr_int = find_first(Some(payload), "rtcp-fb-trr-int");

        if let Some(fb_ele_trr_int) = fb_ele_trr_int {
            let val = fb_ele_trr_int.get_attribute("value").unwrap_or("0");
            sdp.push_str(format!("a=rtcp-fb:* trr-int {}\r\n", val).as_str());
        }

        find_all(Some(payload), "rtcp-fb").iter().for_each(|fb| {
            let fb_stanza = get_attribute!(fb, {
                kind => "type",
                subtype => "subtype"
            });

            sdp.push_str(format!("a=rtcp-fb:{} {}", attr, fb_stanza.kind).as_str());

            if !fb_stanza.subtype.is_empty() {
                sdp.push_str(format!(" {}", fb_stanza.subtype).as_str());
            }
            sdp.push_str("\r\n");
        });

        sdp
    }

    fn adjust_msid_semantic(&self, msid: &str, idx: &str) -> String {
        if self.media == "audio" {
            return msid.to_string();
        }

        let msid_parts = msid.split(" ").collect::<Vec<&str>>();

        if msid_parts.len() == 2 {
            return msid.to_string();
        }

        format!("{} {}-{}", msid, msid, idx)
    }

    fn set_fmt(&mut self, fmt: Vec<String>) {
        self.fmt = fmt;
    }
}
