pub mod jingle_media;

use std::ops::Deref;

use crate::{jingle::jingle_media::JingleMedia, sdp_util::find_all};
use chrono::Utc;
use libstrophe::Stanza;
use thiserror::Error;
use webrtc_sdp::{
    attribute_type::{
        SdpAttribute, SdpAttributeGroup, SdpAttributeGroupSemantic, SdpAttributeMsidSemantic,
        SdpAttributeSsrc, SdpAttributeType, SdpSsrcGroupSemantic,
    },
    error::{SdpParserError, SdpParserInternalError},
    media_type::{SdpMedia, SdpMediaValue},
    parse_sdp,
};

#[derive(Debug, Error)]
pub enum JingleSdpParserError {
    #[error(transparent)]
    SdpParserInternalError(#[from] SdpParserInternalError),
    #[error(transparent)]
    SdpParserError(#[from] SdpParserError),
}

pub fn from_jingle(jingle: &Stanza) -> Result<String, JingleSdpParserError> {
    let mut sdp = String::new();

    let session_id = Utc::now().timestamp_millis();

    sdp.push_str("v=0\r\n");
    sdp.push_str("o=- ");
    sdp.push_str(session_id.to_string().as_str());
    sdp.push_str(" 2 IN IP4 0.0.0.0\r\n");
    sdp.push_str("s=-\r\n");
    sdp.push_str("t=0 0\r\n");

    let groups = find_all(Some(jingle), "group");
    let fingerprints = find_all(Some(jingle), "content>transport>fingerprint");
    let has_cryptex = fingerprints
        .iter()
        .any(|x| x.get_attribute("cryptex") == Some("true"));

    if has_cryptex {
        sdp.push_str("a=cryptex\r\n");
    }

    find_all(Some(jingle), "content")
        .iter()
        .for_each(|content| {
            let mut jingle_media = JingleMedia::new();
            jingle_media.jingle_2_media(content, &mut sdp);
        });

    let mut new_sdp = parse_sdp(sdp.as_str(), true);
    let mut new_media: Vec<SdpMedia> = vec![];

    if let Ok(ref mut new_sdp) = new_sdp {
        for media_line in new_sdp.media.iter() {
            let line_type = media_line.get_type();

            if line_type.clone() == SdpMediaValue::Application {
                let mut new_line = media_line.clone();
                let res = new_line.set_attribute(SdpAttribute::Mid(new_media.len().to_string()));
                if let Ok(()) = res {
                    new_media.push(new_line);
                }
                continue;
            }

            if media_line.get_attribute(SdpAttributeType::Ssrc).is_none() {
                let mut new_line = media_line.clone();
                let res = new_line.set_attribute(SdpAttribute::Mid(new_media.len().to_string()));
                if let Ok(()) = res {
                    new_media.push(new_line);
                }
                continue;
            }

            let mut ssrcs: Vec<SdpAttributeSsrc> = media_line
                .get_attributes_of_type(SdpAttributeType::Ssrc)
                .iter()
                .filter_map(|a| {
                    if let SdpAttribute::Ssrc(s) = a {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
                .collect();

            for (idx, ssrc) in ssrcs.iter_mut().enumerate() {
                if new_media.iter().any(|mline| {
                    mline.get_attributes().iter().any(
                        |attr| matches!(attr, SdpAttribute::Ssrc(source) if source.id == ssrc.id),
                    )
                }) {
                    continue;
                }
                let mut new_line = media_line.clone();
                new_line.remove_attribute(SdpAttributeType::Ssrc);
                new_line.remove_attribute(SdpAttributeType::SsrcGroup);
                new_line.remove_attribute(SdpAttributeType::Sendonly);
                new_line.remove_attribute(SdpAttributeType::Sendrecv);
                if let Err(err) =
                    new_line.set_attribute(SdpAttribute::Mid(new_media.len().to_string()))
                {
                    return Err(JingleSdpParserError::SdpParserInternalError(err));
                };
                if idx == 0 {
                    if let Err(err) = new_line.add_attribute(SdpAttribute::Sendrecv) {
                        return Err(JingleSdpParserError::SdpParserInternalError(err));
                    };
                } else {
                    if let Err(err) = new_line.add_attribute(SdpAttribute::Sendonly) {
                        return Err(JingleSdpParserError::SdpParserInternalError(err));
                    };
                }

                let ssrc_id = ssrc.id;
                let group: Option<(&SdpSsrcGroupSemantic, &Vec<SdpAttributeSsrc>)> = media_line
                    .get_attributes()
                    .iter()
                    .filter_map(|attr| {
                        if let SdpAttribute::SsrcGroup(semantic, ssrcs) = attr {
                            Some((semantic, ssrcs))
                        } else {
                            None
                        }
                    })
                    .find(|(_, ssrcs)| ssrcs.iter().any(|ssrc| ssrc.id == ssrc_id));

                if let Some(g) = group {
                    if ssrc.attribute.as_deref() == Some("msid") {
                        if let Some(SdpAttribute::Mid(m)) =
                            new_line.get_attribute(SdpAttributeType::Mid)
                        {
                            if let Some(value) = &ssrc.value {
                                ssrc.value = Some(JingleMedia::adjust_msid_semantic(
                                    value,
                                    line_type.to_string().as_str(),
                                    m,
                                ));
                            }
                        }
                    }

                    if let Err(err) = new_line.add_attribute(SdpAttribute::Ssrc(SdpAttributeSsrc {
                        id: ssrc.id,
                        attribute: ssrc.deref().attribute.clone(),
                        value: ssrc.value.clone(),
                    })) {
                        return Err(JingleSdpParserError::SdpParserInternalError(err));
                    }

                    let other_ssrc = g.1.iter().find(|o_ssrc| o_ssrc.id != ssrc_id);

                    if let Some(other_ssrc) = other_ssrc {
                        let mut other_source: Option<SdpAttributeSsrc> = media_line
                            .get_attributes_of_type(SdpAttributeType::Ssrc)
                            .iter()
                            .filter_map(|a| {
                                if let SdpAttribute::Ssrc(s) = a {
                                    Some(s.clone())
                                } else {
                                    None
                                }
                            })
                            .find(|source| source.id == other_ssrc.id);

                        if let Some(ref mut other_source) = other_source
                            && other_source.attribute.as_deref() == Some("msid")
                        {
                            if let Some(SdpAttribute::Mid(m)) =
                                new_line.get_attribute(SdpAttributeType::Mid)
                            {
                                if let Some(ref value) = other_source.value {
                                    other_source.value = Some(JingleMedia::adjust_msid_semantic(
                                        value.as_str(),
                                        line_type.to_string().as_str(),
                                        m,
                                    ))
                                }
                            }
                        }

                        if let Some(ref other) = other_source {
                            if let Err(err) =
                                new_line.add_attribute(SdpAttribute::Ssrc(SdpAttributeSsrc {
                                    id: other.id,
                                    attribute: other.attribute.clone(),
                                    value: other.value.clone(),
                                }))
                            {
                                return Err(JingleSdpParserError::SdpParserInternalError(err));
                            }
                        }
                    }

                    if let Err(err) =
                        new_line.add_attribute(SdpAttribute::SsrcGroup(g.0.clone(), g.1.clone()))
                    {
                        return Err(JingleSdpParserError::SdpParserInternalError(err));
                    }
                } else {
                    if let Err(err) = new_line.add_attribute(SdpAttribute::Ssrc(SdpAttributeSsrc {
                        id: ssrc.id,
                        attribute: ssrc.attribute.clone(),
                        value: ssrc.value.clone(),
                    })) {
                        return Err(JingleSdpParserError::SdpParserInternalError(err));
                    }
                }

                new_media.push(new_line);
            }
        }

        new_sdp.media = new_media.clone();
        let mut mids = vec![];

        new_media.iter().for_each(|media| {
            if let Some(SdpAttribute::Mid(m)) = media.get_attribute(SdpAttributeType::Mid) {
                mids.push(m.clone());
            }
        });

        if groups.len() > 0 {
            if let Err(err) = new_sdp.add_attribute(SdpAttribute::Group(SdpAttributeGroup {
                semantics: SdpAttributeGroupSemantic::Bundle,
                tags: mids,
            })) {
                return Err(JingleSdpParserError::SdpParserInternalError(err));
            };
        }

        let msids: Vec<String> = new_media
            .iter()
            .flat_map(|media| media.get_attributes())
            .filter_map(|attr| {
                if let SdpAttribute::Ssrc(ssrc) = attr {
                    if ssrc.attribute.as_deref() == Some("msid") {
                        ssrc.value
                            .as_ref()
                            .and_then(|v| v.split_whitespace().next().map(|s| s.to_string()))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect::<std::collections::HashSet<_>>() // Remove duplicates
            .into_iter()
            .collect();

        if let Err(err) =
            new_sdp.add_attribute(SdpAttribute::MsidSemantic(SdpAttributeMsidSemantic {
                semantic: "WMS".to_string(),
                msids: msids,
            }))
        {
            return Err(JingleSdpParserError::SdpParserInternalError(err));
        };
    }

    match new_sdp {
        Ok(sdp_sess) => Ok(sdp_sess.to_string()),
        Err(err) => {
            return Err(JingleSdpParserError::SdpParserError(err));
        }
    }
}
