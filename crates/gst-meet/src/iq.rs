pub(crate) mod jingle_action;
pub(crate) mod jingle_media;

use crate::{
    get_attribute,
    iq::{jingle_action::JingleAction, jingle_media::JingleMedia},
    make_stanza,
    room_manager::Rooms,
    set_attribute,
};
use libstrophe::{Error, Stanza};
use nanoid::nanoid;
use webrtc_sdp::{address::Address, attribute_type::SdpAttributeCandidate};

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
                    let jitsi_offer =
                        action.handle_session_initiate(stanza, &mut self.jingle_media);
                }
                JingleAction::SourceAdd(stanza) => {
                    action.handle_source_add(stanza);
                }
            }
        }
    }

    pub fn handle_query(&self, _stanza: &Stanza) {}

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
