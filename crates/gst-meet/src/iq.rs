//! IQ handling: Jingle session negotiation and service discovery.
//!
//! The focus drives the call over Jingle IQs. This module answers them: it
//! converts `session-initiate` into an SDP offer for `webrtcbin` (through
//! `jingle_media` and [`crate::sdp`]), records the SSRC-to-source mapping
//! from `source-add`, and replies to disco#info with the feature set that makes
//! Jitsi treat the recorder as a full participant.

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

/// One incoming IQ stanza, plus the media state built while answering it.
#[derive(Debug, Clone)]
#[allow(unused)]
pub struct Iq {
    pub id: String,
    pub from: String,
    pub to: String,
    /// IQ type: `get`, `set`, `result` or `error`.
    pub kind: String,
    /// Codec and header-extension state accumulated from the Jingle
    /// description, used to build the SDP offer.
    jingle_media: JingleMedia,
}

impl Iq {
    /// Reads the routing attributes off an IQ stanza.
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

    /// Wires the room's ICE candidates to this Jingle session, so candidates
    /// gathered by `webrtcbin` are sent as `transport-info` for the right sid.
    pub fn initialize_meeting(&self, room_manager: Rooms, sid: String, initiator: String) {
        let room_name = self.from.split('@').next().unwrap_or_default();

        match room_manager.lock() {
            Ok(mut room_manager) => {
                room_manager.get_mut(room_name).and_then(|room| {
                    room.handle_ice_candidate(&self.from, &self.to, &sid, &initiator);
                    Some(room)
                });
            }
            Err(err) => {
                error!("failed to get mutext guard lock for jingle: {err:?}");
            }
        }
    }

    /// Dispatches a `<jingle>` payload by action.
    ///
    /// `session-initiate` is converted to an SDP offer and handed to the
    /// room's `webrtcbin`; `source-add` registers new SSRCs (which may release
    /// pads the room has parked). Every handled action is acked.
    pub fn handle_jingle(&mut self, stanza: &Stanza, room_manager: Rooms, tx: Sender<Stanza>) {
        let jingle_stanza = get_attribute!(stanza, [sid, initiator, action]);
        let room_name = self.from.split('@').next().unwrap_or_default();
        let jingle_action = JingleAction::parse(&jingle_stanza.action, stanza);
        if let Some(ref action) = jingle_action {
            match action {
                JingleAction::SessionInitiate(stanza) => {
                    self.initialize_meeting(
                        room_manager.clone(),
                        jingle_stanza.sid,
                        jingle_stanza.initiator,
                    );
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
                        room.handle_session_initiate(stanza, &self.from, &self.to, sdp_message);
                        // room.on_meeting_started();
                        let sources = action.handle_source_add(&stanza);
                        for parsed_source in sources {
                            room.handle_register_ssrc(parsed_source);
                        }
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
                    let sources = action.handle_source_add(stanza);
                    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                        let mut room_manager_lock = room_manager.lock()?;
                        let room = room_manager_lock
                            .get_mut(room_name)
                            .ok_or_else(|| format!("no room found for: {}", room_name))?;

                        for parsed_source in sources {
                            room.handle_register_ssrc(parsed_source);
                        }
                        Ok(())
                    })();

                    match result {
                        Ok(_) => {
                            info!("successfully processed session-add for room: {}", room_name);
                        }

                        Err(err) => {
                            error!(
                                "failed to process session-add for room {}: {:?}",
                                room_name, err
                            );
                        }
                    }
                }

                JingleAction::SourceRemove(stanza) => {
                    action.handle_source_remove(stanza);
                }
            }
            if self.kind == "set" {
                match self.handle_ack(tx.clone()) {
                    Ok(_) => {
                        info!("successfully sent ack response for: {} room", room_name);
                    }
                    Err(err) => {
                        error!(
                            "failed to send ack response for: {} room | err: {:?}",
                            room_name, err
                        )
                    }
                }
            }
        }
    }

    /// Queues the empty `type="result"` reply that acknowledges this IQ.
    pub fn handle_ack(&self, tx: Sender<Stanza>) -> Result<(), Box<dyn std::error::Error>> {
        let iq_stanza = make_stanza!("iq", {
            "id" => &self.id,
            "type" => "result",
            "from" => &self.to,
            "to" => &self.from

        })?;

        tx.send(iq_stanza)?;
        Ok(())
    }

    /// Answers a disco#info query with the recorder's identity and features.
    ///
    /// The advertised feature list is what makes Jitsi negotiate the modern
    /// stack with us — JSON-encoded sources, source names, and multiple video
    /// streams. Non-disco queries are ignored.
    pub fn handle_query(&self, stanza: &Stanza, tx: Sender<Stanza>) -> Result<(), Error> {
        let is_disco_info = stanza.name() == Some("query")
            && stanza.ns() == Some("http://jabber.org/protocol/disco#info");

        if !is_disco_info {
            error!("query is not a disco info");
            return Ok(());
        }

        let room_name = self.from.split('@').next().unwrap_or_default();

        if self.kind == "set" {
            match self.handle_ack(tx.clone()) {
                Ok(_) => {
                    info!("successfully sent ack response for: {} room", room_name);
                }
                Err(err) => {
                    error!(
                        "failed to send ack response for: {} room | err: {:?}",
                        room_name, err
                    )
                }
            }
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
            "https://jitsi.org/meet/e2ee",
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
            "type" => "result",
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

    /// Converts an SDP ICE candidate into the Jingle `<candidate>` element
    /// carried in `transport-info`.
    pub fn parse_candidate(candidate: &SdpAttributeCandidate) -> Result<Stanza, Error> {
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
