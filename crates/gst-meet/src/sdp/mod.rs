use libstrophe::{Error, Stanza};
use webrtc_sdp::{
    SdpSession,
    attribute_type::{SdpAttribute, SdpAttributeType},
    media_type::SdpFormatList,
};

use crate::{
    jingle::Jingle,
    sdp::sdp_util::{
        find_fmtp, find_rtpmap, fmtp_to_params, rtcp_fb_to_jingle, rtcp_mux_exist,
        transport_to_jingle,
    },
    xmpp::xep::XEP,
};

pub mod sdp_util;

pub fn to_jingle(sdp_session: &SdpSession, jingle: Jingle) -> Result<Stanza, Error> {
    let mut jingle_stanza = Stanza::new();
    jingle_stanza.set_name("jingle")?;
    jingle_stanza.set_attribute("action", "session-accept")?;
    jingle_stanza.set_ns("urn:xmpp:jingle:1")?;
    jingle_stanza.set_attribute("initiator", jingle.get_initiator())?;
    jingle_stanza.set_attribute("sid", jingle.get_sid())?;
    jingle_stanza.set_attribute("responder", jingle.get_responder())?;

    let mut group_stanza = Stanza::new();

    let sdp_group = sdp_session.get_attribute(SdpAttributeType::Group);
    if let Some(group) = sdp_group {
        if let SdpAttribute::Group(group_attr) = group {
            group_stanza.set_name("group")?;
            group_stanza.set_attribute("semantics", group_attr.semantics.to_string())?;
            group_stanza.set_ns(XEP::BundleMedia.to_string())?;
        }

        jingle_stanza.add_child(group_stanza)?;
    }

    for media in sdp_session.media.iter() {
        let media_type = media.get_type().to_string();

        let mut content_stanza = Stanza::new();
        content_stanza.set_name("content")?;
        content_stanza.set_attribute("name", media_type.to_string())?;
        content_stanza.set_attribute("creator", "responder")?;
        content_stanza.set_attribute("senders", "responder")?;

        if ["audio", "video"].contains(&media_type.as_str()) {
            let mut description_stanza = Stanza::new();
            description_stanza.set_name("description")?;
            description_stanza.set_attribute("media", media_type)?;
            description_stanza.set_ns(XEP::RtpMedia.to_string())?;

            match media.get_formats() {
                //TODO: check out this comment letter on
                // SdpFormatList::Strings(formats) => for format in formats {},
                SdpFormatList::Integers(formats) => {
                    for format in formats {
                        let rtpmap = find_rtpmap(media, format.clone());

                        if let Some(rtpmap) = rtpmap {
                            let mut payload_type_stanza = Stanza::new();

                            payload_type_stanza.set_name("payload-type")?;
                            payload_type_stanza
                                .set_attribute("id", rtpmap.payload_type.to_string())?;
                            payload_type_stanza
                                .set_attribute("clockrate", rtpmap.frequency.to_string())?;
                            payload_type_stanza.set_attribute(
                                "channels",
                                rtpmap.channels.unwrap_or(1).to_string(),
                            )?;
                            payload_type_stanza
                                .set_attribute("name", rtpmap.codec_name.to_string())?;

                            let fmtp = find_fmtp(media, format.clone());

                            if let Some(fmtp) = fmtp {
                                for param in fmtp_to_params(&fmtp.parameters) {
                                    let mut parameter_stanza = Stanza::new();
                                    parameter_stanza.set_name("parameter")?;
                                    parameter_stanza.set_attribute(param.0, param.1)?;
                                    payload_type_stanza.add_child(parameter_stanza)?;
                                }
                            }

                            rtcp_fb_to_jingle(media, &mut payload_type_stanza)?;

                            description_stanza.add_child(payload_type_stanza)?;
                        }
                    }
                }
                _ => {}
            }

            if rtcp_mux_exist(media) {
                let mut rtcp_mux_stanza = Stanza::new();
                rtcp_mux_stanza.set_name("rtcp-mux")?;
                description_stanza.add_child(rtcp_mux_stanza)?;
            }

            let mut transport_stanza = Stanza::new();
            transport_stanza.set_name("transport")?;
            transport_to_jingle(media, &mut transport_stanza)?;

            content_stanza.add_child(description_stanza)?;
            content_stanza.add_child(transport_stanza)?;
        }
        jingle_stanza.add_child(content_stanza)?;
    }

    Ok(jingle_stanza)
}
