use libstrophe::Stanza;

use crate::jingle::jingle_util::{exists, find_all, find_first};

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

    fn set_media(&mut self, attr: Option<&str>) {
        if let Some(media) = attr {
            self.media = Some(media.to_string());
        } else {
            self.media = None;
        }
    }

    fn set_port(&mut self, attr: Option<&str>) {
        if let Some(senders) = attr
            && senders == "rejected"
        {
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

    fn set_fmt(&mut self, fmt: Vec<String>) {
        self.fmt = fmt;
    }

    fn set_senders(&self, content: &Stanza, sdp: &mut String) {
        let senders = content.get_attribute("senders").unwrap_or_default();
        match senders {
            "initiator" => sdp.push_str("a=recvonly\r\n"),
            "responder" => sdp.push_str("a=sendonly\r\n"),
            "none" => sdp.push_str("a=inactive\r\n"),
            "both" => sdp.push_str("a=sendrecv\r\n"),
            _ => {}
        }
    }

    pub fn candidate_from_jingle(&self, candidate: &Stanza, sdp: &mut String) {
        sdp.push_str("a=candidate:");

        let foundation = candidate.get_attribute("foundation").unwrap_or_default();
        let component = candidate.get_attribute("component").unwrap_or_default();
        let protocol = candidate.get_attribute("protocol").unwrap_or_default();
        let priority = candidate.get_attribute("priority").unwrap_or_default();
        let ip = candidate.get_attribute("ip").unwrap_or_default();
        let port = candidate.get_attribute("port").unwrap_or_default();
        let cand_type = candidate.get_attribute("type").unwrap_or_default();
        let rel_addr = candidate.get_attribute("rel-addr");
        let rel_port = candidate.get_attribute("rel-port");

        sdp.push_str(
            format!(
                "{} {} {} {} {} {} typ {} ",
                foundation, component, protocol, priority, ip, port, cand_type
            )
            .as_str(),
        );

        match cand_type {
            "srflx" | "prflx" | "relay" => {
                if rel_port.is_some() && rel_addr.is_some() {
                    sdp.push_str(
                        format!(
                            "raddr {} rport {} ",
                            rel_addr.unwrap_or_default(),
                            rel_port.unwrap_or_default()
                        )
                        .as_str(),
                    );
                }
            }
            _ => {}
        }

        if protocol.to_lowercase() == "tcp" {
            sdp.push_str(
                format!(
                    "tcptype {} ",
                    candidate.get_attribute("tcptype").unwrap_or_default(),
                )
                .as_str(),
            );
        }

        sdp.push_str(
            format!(
                "generation {}",
                candidate.get_attribute("generation").unwrap_or("0")
            )
            .as_str(),
        );

        sdp.push_str("\r\n");
    }

    fn build_rtp_map(&self, rtp_map: &Stanza, sdp: &mut String) {
        let id = rtp_map.get_attribute("id").unwrap_or_default();
        let name = rtp_map.get_attribute("name").unwrap_or_default();
        let clockrate = rtp_map.get_attribute("clockrate").unwrap_or_default();
        let channels = rtp_map.get_attribute("channels");

        sdp.push_str(format!("a=rtpmap:{} {}/{}", id, name, clockrate).as_str());

        if let Some(chan) = channels
            && chan != "1"
        {
            sdp.push_str("/");
            sdp.push_str(chan);
        }

        sdp.push_str("\r\n");
    }

    fn rtcp_fb_from_jingle(&self, payload: &Stanza, sdp: &mut String, attr: &str) {
        let fb_ele_trr_int = find_first(Some(payload), "rtcp-fb-trr-int");

        if let Some(fb_ele_trr_int) = fb_ele_trr_int {
            let val = fb_ele_trr_int.get_attribute("value").unwrap_or("0");
            sdp.push_str(format!("a=rtcp-fb:* trr-int {}\r\n", val).as_str());
        }

        find_all(Some(payload), "rtcp-fb").iter().for_each(|fb| {
            let fb_type = fb.get_attribute("type").unwrap_or_default();
            sdp.push_str(format!("a=rtcp-fb:{} {}", attr, fb_type).as_str());

            if let Some(subtype) = fb.get_attribute("subtype") {
                sdp.push_str(format!(" {}", subtype).as_str());
            }
            sdp.push_str("\r\n");
        });
    }

    pub fn adjust_msid_semantic(msid: &str, media: &str, idx: &str) -> String {
        if media == "audio" {
            return msid.to_string();
        }

        let msid_parts = msid.split(" ").collect::<Vec<&str>>();

        if msid_parts.len() == 2 {
            return msid.to_string();
        }

        format!("{} {}-{}", msid, msid, idx)
    }

    pub fn jingle_2_media(&mut self, content: &Stanza, sdp: &mut String) {
        let desc = find_first(Some(&content), "description");
        let transport = find_first(Some(&content), "transport");
        let sctp = find_first(transport.as_ref(), "sctpmap");
        let mid = content.get_attribute("name").unwrap_or_default();
        self.set_media(content.get_attribute("name"));
        self.set_port(content.get_attribute("senders"));
        let fing_exists = exists(transport.as_ref(), "fingerprint");
        self.set_proto(fing_exists, sctp.as_ref());

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
                .map(|payload_type| {
                    if let Some(id) = payload_type.get_attribute("id") {
                        return id.to_string();
                    }
                    String::new()
                })
                .filter(|id| !id.is_empty())
                .collect();
            self.set_fmt(fmt);
            sdp.push_str(
                format!(
                    "m={} {} {} {}",
                    self.media.as_deref().unwrap_or(""),
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
                    self.candidate_from_jingle(cand, sdp);
                });
        }

        self.set_senders(content, sdp);
        sdp.push_str(format!("a=mid:{}\r\n", mid).as_str());

        if exists(desc.as_ref(), "rtcp-mux") {
            sdp.push_str("a=rtcp-mux\r\n");
        }

        if let Some(desc) = desc {
            find_all(Some(&desc), "payload-type")
                .iter()
                .for_each(|payload_type| {
                    self.build_rtp_map(payload_type, sdp);
                    let parameters = find_all(Some(payload_type), "parameter");
                    let id = payload_type.get_attribute("id").unwrap_or_default();
                    if parameters.len() > 0 {
                        sdp.push_str(format!("a=fmtp:{} ", id).as_str());
                        let params = parameters
                            .iter()
                            .map(|param| {
                                let value = param.get_attribute("value").unwrap_or_default();

                                match param.get_attribute("name") {
                                    Some(name) => format!("{}={}", name, value),
                                    None => value.to_string(),
                                }
                            })
                            .collect::<Vec<String>>()
                            .join(";");

                        sdp.push_str(params.as_str());
                        sdp.push_str("\r\n");
                    }

                    self.rtcp_fb_from_jingle(payload_type, sdp, id);
                });

            self.rtcp_fb_from_jingle(&desc, sdp, "*");

            find_all(Some(&desc), "rtp-hdrext")
                .iter()
                .for_each(|hdr_ext| {
                    let id = hdr_ext.get_attribute("id").unwrap_or_default();
                    let uri = hdr_ext.get_attribute("uri").unwrap_or_default();
                    sdp.push_str(format!("a=extmap:{} {}\r\n", id, uri).as_str());
                });

            if exists(Some(&desc), "extmap-allow-mixed") {
                sdp.push_str("a=extmap-allow-mixed\r\n");
            }

            find_all(Some(&desc), "ssrc-group")
                .iter()
                .for_each(|ssrc_group| {
                    let semantics = ssrc_group.get_attribute("semantics").unwrap_or_default();
                    let ssrcs: Vec<String> = ssrc_group
                        .children()
                        .filter_map(|child| {
                            (child.name() == Some("source"))
                                .then(|| child.get_attribute("ssrc"))
                                .flatten()
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

            find_all(Some(&desc), "source").iter().for_each(|source| {
                let ssrc = source.get_attribute("ssrc").unwrap_or_default();
                let mut is_user_source = true;
                let mut source_str = String::new();

                find_all(Some(source), "parameter")
                    .iter()
                    .for_each(|parameter| {
                        let name = parameter.get_attribute("name").unwrap_or_default();
                        let mut value = parameter
                            .get_attribute("value")
                            .unwrap_or_default()
                            .to_string();

                        value = value
                            .chars()
                            .filter(|c| !matches!(c, '\\' | '/' | '{' | '}' | ',' | '+'))
                            .collect();

                        source_str.push_str(format!("a=ssrc:{} {}", ssrc, name).as_str());

                        if name == "msid" {
                            let media = self.media.as_deref().unwrap_or("");
                            value = JingleMedia::adjust_msid_semantic(value.as_str(), media, mid);
                        }

                        if value.len() > 0 {
                            source_str.push_str(format!(":{}", value).as_str());
                        }
                        source_str.push_str("\r\n");

                        if value.contains("mixedmslabel") {
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
        }
    }
}
