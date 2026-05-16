pub(crate) mod jingle_action;
pub(crate) mod jingle_media;

use std::sync::mpsc::Sender;

use crate::{
    get_attribute,
    iq::{jingle_action::JingleAction, jingle_media::JingleMedia},
    make_stanza,
    room_manager::Rooms,
    set_attribute,
};
use gstreamer_sdp::SDPMessage;
use libstrophe::{Error, Stanza};
use log::{error, info};
use nanoid::nanoid;
use webrtc_sdp::{address::Address, attribute_type::SdpAttributeCandidate, parse_sdp};

#[derive(Debug, Clone)]
#[allow(unused)]
pub struct Iq {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: String,
    jingle_media: JingleMedia,
}

impl Iq {
    pub fn new(stanza: &Stanza) -> Self {
        let iq_stanza = get_attribute!(stanza, {
            from => "from",
            to => "to",
            id => "id",
            kind => "type"
        });

        let media = JingleMedia::new();

        Self {
            id: iq_stanza.id,
            from: iq_stanza.from,
            to: iq_stanza.to,
            kind: iq_stanza.kind,
            jingle_media: media,
        }
    }

    pub fn handle_jingle(&mut self, stanza: &Stanza, room_manager: Rooms) {
        let jingle_stanza = get_attribute!(stanza, [sid, initiator, action]);
        let jingle_action = JingleAction::parse(&jingle_stanza.action, stanza);
        if let Some(ref action) = jingle_action {
            match action {
                JingleAction::SessionInitiate(stanza) => {
                    let room_name = self.from.split('@').next().unwrap_or_default();
                    let jitsi_offer =
                        action.handle_session_initiate(stanza, &mut self.jingle_media);

                    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                        let mut room_manager_lock = room_manager.lock()?;
                        let room = room_manager_lock
                            .get_mut(room_name)
                            .ok_or_else(|| format!("no room found for: {}", room_name))?;
                        let sdp_offer = parse_sdp(&jitsi_offer, true)?;
                        let sdp_message =
                            SDPMessage::parse_buffer(sdp_offer.to_string().as_bytes())?;
                        room.handle_session_initiate(stanza, sdp_message);
                        Ok(())
                    })();

                    match result {
                        Ok(_) => {
                            info!(
                                "successfully processed session-initiate for room: {}",
                                room_name
                            );
                        }

                        Err(err) => {
                            error!(
                                "failed to process session-initiate for room {}: {:?}",
                                room_name, err
                            );
                        }
                    }
                }
                JingleAction::SourceAdd(stanza) => {
                    action.handle_source_add(stanza);
                }
            }
        }
    }

    pub fn handle_query(&self, stanza: &Stanza, tx: Sender<Stanza>) -> Result<(), Error> {
        let is_disco_info = stanza.name() == Some("query")
            && stanza.ns() == Some("http://jabber.org/protocol/disco#info");

        if !is_disco_info {
            error!("query is not a disco info");
            return Ok(());
        }

        let features = [
            "urn:xmpp:jingle:1",
            "urn:xmpp:jingle:apps:rtp:1",
            "urn:xmpp:jingle:transports:ice-udp:1",
            "urn:xmpp:jingle:apps:dtls:0",
            "urn:xmpp:jingle:transports:dtls-sctp:1",
            "urn:xmpp:jingle:apps:rtp:audio",
            "urn:xmpp:jingle:apps:rtp:video",
            "http://jitsi.org/json-encoded-sources",
            "http://jitsi.org/source-name",
            "http://jitsi.org/receive-multiple-video-streams",
            "urn:ietf:rfc:4588",
        ];

        let identity_stanza = make_stanza!("identity", {
            "category" => "client",
            "type" => "pc",
            "name" => "gst-meet-record",
        })?;

        let mut query_stanza = make_stanza!("query", {
            "xmlns" => "http://jabber.org/protocol/disco#info"
        }, [identity_stanza])?;

        for feature_ns in features {
            let feature_stanza = make_stanza!("feature", {
                "var" => feature_ns,
            })?;
            query_stanza.add_child(feature_stanza)?;
        }

        let iq_stanza = make_stanza!("iq", {
            "id" => &self.id,
            "to" => &self.from,
            "from" => &self.to,
        }, [query_stanza])?;

        match tx.send(iq_stanza) {
            Ok(_) => {
                info!("successfully sent query stanza");
            }
            Err(err) => {
                error!("failed to send query stanza: {:?}", err)
            }
        }

        Ok(())
    }

    pub fn parse_candidate(&self, candidate: &SdpAttributeCandidate) -> Result<Stanza, Error> {
        let mut candidate_stanza = make_stanza!("candidate", {
            "port" => candidate.port.to_string(),
            "component" => candidate.component.to_string(),
            "foundation" => candidate.foundation.to_string(),
            "type" => candidate.c_type.to_string(),
            "generation" => candidate.generation.unwrap_or_default().to_string(),
            "network" => "1",
            "id" => nanoid!(),
            "protocol" => candidate.transport.to_string().to_lowercase(),
            "priority" => candidate.priority.to_string()
        })?;

        match candidate.address.clone() {
            Address::Ip(ip) => {
                set_attribute!(candidate_stanza, {"ip" => ip.to_string()})?;
            }
            Address::Fqdn(fqdn) => {
                set_attribute!(candidate_stanza, {"ip" => fqdn})?;
            }
        }

        if let Some(raddr) = candidate.raddr.clone() {
            match raddr {
                Address::Ip(ip) => {
                    set_attribute!(candidate_stanza, {"rel-addr" => ip.to_string()})?;
                }

                Address::Fqdn(fqdn) => {
                    set_attribute!(candidate_stanza, {"rel-addr" => fqdn})?;
                }
            }
        }

        if let Some(rport) = candidate.rport {
            set_attribute!(candidate_stanza, {"rel-port" => rport.to_string()})?;
        }

        Ok(candidate_stanza)
    }
}
