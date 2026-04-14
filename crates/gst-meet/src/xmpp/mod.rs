use std::sync::{Arc, mpsc::Sender};

use libstrophe::{Error, Stanza};
use log::{info, warn};

use crate::{
    gst::webrtcbin,
    jingle::{Iq, Jingle, from_jingle},
};

pub mod xep;

pub fn ack_session_initiate(iq_stanza: &Stanza) -> Result<Stanza, Error> {
    let id = iq_stanza.get_attribute("id");
    let to = iq_stanza.get_attribute("to").unwrap_or_default();
    let from = iq_stanza.get_attribute("from").unwrap_or_default();
    let mut iq = Stanza::new_iq(Some("result"), id);
    iq.set_attribute("to", from)?;
    iq.set_attribute("from", to)?;

    return Ok(iq);
}

pub fn handle_jingle_request(
    child: &Stanza,
    tx: Sender<Stanza>,
    iq_id: &str,
    iq_to: &str,
    iq_from: &str,
) {
    let action = child.get_attribute("action").unwrap_or_default();
    match action {
        "session-initiate" => {
            info!("got session initiate request");
            match from_jingle(child) {
                Ok(sdp) => {
                    let sid = child.get_attribute("sid");
                    let initiator = child.get_attribute("initiator");

                    let jingle = Arc::new(Jingle::new(
                        sid.unwrap_or_default().to_string(),
                        initiator.unwrap_or_default().to_string(),
                        iq_to.to_string(),
                    ));
                    let iq = Arc::new(Iq::new(
                        iq_id.to_string(),
                        iq_to.to_string(),
                        iq_from.to_string(),
                    ));
                    let res = webrtcbin(&sdp, jingle, iq, tx.clone());
                    match res {
                        Err(err) => warn!("Error occured: {:?}", err),
                        _ => {}
                    }
                }
                Err(err) => {
                    warn!("Failed to generate sdp: {}", err);
                }
            }
        }
        "source-add" => {
            info!("source-added: {}", child.to_string());
        }

        _ => {}
    }
}

pub fn handle_query_request(
    child: &Stanza,
    tx: Sender<Stanza>,
    iq_id: &str,
    iq_to: &str,
    iq_from: &str,
) {
    let is_disco_info = child.name() == Some("query")
        && child.ns() == Some("http://jabber.org/protocol/disco#info");

    if !is_disco_info {
        return;
    }

    let mut iq_result = Stanza::new_iq(Some("result"), Some(iq_id));
    iq_result
        .set_attribute("to", iq_from)
        .expect("failed to set iq to");
    iq_result
        .set_attribute("from", iq_to)
        .expect("failed to set iq from");

    let mut query = Stanza::new();
    query.set_name("query").expect("failed to set query name");
    query
        .set_ns("http://jabber.org/protocol/disco#info")
        .expect("failed to set query ns");

    let mut identity = Stanza::new();
    identity
        .set_name("identity")
        .expect("failed to set identity name");
    identity
        .set_attribute("category", "client")
        .expect("failed to set category");
    identity
        .set_attribute("type", "pc")
        .expect("failed to set type");
    identity
        .set_attribute("name", "gst-meet")
        .expect("failed to set name");
    query.add_child(identity).expect("failed to add identity");

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

    for feature_ns in features {
        let mut feature = Stanza::new();
        feature
            .set_name("feature")
            .expect("failed to set feature name");
        feature
            .set_attribute("var", feature_ns)
            .expect("failed to set feature var");
        query.add_child(feature).expect("failed to add feature");
    }

    iq_result
        .add_child(query)
        .expect("failed to add query to iq result");

    tx.send(iq_result).expect("failed to send disco#info reply");
}
