use std::fmt::Display;

use chrono::Utc;
use libstrophe::Stanza;
use log::{debug, warn};
use webrtc_sdp::{
    attribute_type::{
        SdpAttribute, SdpAttributeGroup, SdpAttributeGroupSemantic, SdpAttributeMsidSemantic,
        SdpAttributeSsrc, SdpAttributeType, SdpSsrcGroupSemantic,
    },
    media_type::{SdpMedia, SdpMediaValue},
    parse_sdp,
};

use crate::{iq::jingle_media::JingleMedia, util::find_all};

#[derive(Debug)]
pub enum JingleAction<'a> {
    SessionInitiate(&'a Stanza),
    SourceAdd(&'a Stanza),
}

impl<'a> JingleAction<'a> {
    pub(crate) fn parse(s: &str, stanza: &'a Stanza) -> Option<Self> {
        match s {
            "session-initiate" => Some(Self::SessionInitiate(stanza)),
            "source-add" => Some(Self::SourceAdd(stanza)),
            _ => None,
        }
    }

    pub fn handle_session_initiate(&self, stanza: &Stanza, media: &mut JingleMedia) -> String {
        let mut sdp_session = self.parse_sdp_session(stanza);
        let mut sdp_media = String::from("");
        let groups = find_all(Some(stanza), "group");

        find_all(Some(stanza), "content")
            .iter()
            .for_each(|content| {
                sdp_media.push_str(&media.parse_sdp_media(content));
            });

        sdp_session.push_str(&sdp_media);

        let final_sdp = self
            .final_parsing(&sdp_session, groups, media)
            .unwrap_or_else(|err| {
                warn!("unable to parse jingle to sdp: {err:?}");
                "".to_string()
            });

        debug!("session initiat offer: {}", sdp_session);

        final_sdp
    }

    pub fn handle_source_add(&self, _stanza: &Stanza) -> String {
        String::new()
    }

    fn parse_sdp_session(&self, stanza: &Stanza) -> String {
        let mut sdp = String::new();
        let session_id = Utc::now().timestamp_millis();
        sdp.push_str("v=0\r\n");
        sdp.push_str("o=- ");
        sdp.push_str(session_id.to_string().as_str());
        sdp.push_str(" 2 IN IP4 0.0.0.0\r\n");
        sdp.push_str("s=-\r\n");
        sdp.push_str("t=0 0\r\n");

        let fingerprints = find_all(Some(stanza), "content>transport>fingerprint");
        let has_cryptex = fingerprints
            .iter()
            .any(|x| x.get_attribute("cryptex") == Some("true"));

        if has_cryptex {
            sdp.push_str("a=cryptex\r\n");
        }
        sdp
    }

    fn final_parsing(
        &self,
        sdp: &str,
        groups: Vec<Stanza>,
        media: &mut JingleMedia,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let mut new_sdp = parse_sdp(sdp, true);
        let mut new_media: Vec<SdpMedia> = vec![];

        match new_sdp {
            Ok(ref mut new_sdp) => {
                for media_line in new_sdp.media.iter() {
                    let line_type = media_line.get_type();

                    if line_type.clone() == SdpMediaValue::Application {
                        let mut new_line = media_line.clone();

                        new_line.set_attribute(SdpAttribute::Mid(new_media.len().to_string()))?;

                        new_media.push(new_line);
                        continue;
                    }

                    if media_line.get_attribute(SdpAttributeType::Ssrc).is_none() {
                        let mut new_line = media_line.clone();

                        new_line.set_attribute(SdpAttribute::Mid(new_media.len().to_string()))?;

                        new_media.push(new_line);
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

                        new_line.set_attribute(SdpAttribute::Mid(new_media.len().to_string()))?;

                        if idx == 0 {
                            new_line.add_attribute(SdpAttribute::Sendrecv)?;
                        } else {
                            new_line.add_attribute(SdpAttribute::Sendonly)?;
                        }

                        let ssrc_id = ssrc.id;

                        let group: Option<(&SdpSsrcGroupSemantic, &Vec<SdpAttributeSsrc>)> =
                            media_line
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
                                if let Some(SdpAttribute::Mid(_m)) =
                                    new_line.get_attribute(SdpAttributeType::Mid)
                                {
                                    if let Some(value) = &ssrc.value {
                                        ssrc.value = Some(media.adjust_msid_semantic(
                                            value,
                                            line_type.to_string().as_str(),
                                        ));
                                    }
                                }
                            }

                            new_line.add_attribute(SdpAttribute::Ssrc(SdpAttributeSsrc {
                                id: ssrc.id,
                                attribute: ssrc.attribute.clone(),
                                value: ssrc.value.clone(),
                            }))?;

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
                                    if let Some(SdpAttribute::Mid(_m)) =
                                        new_line.get_attribute(SdpAttributeType::Mid)
                                    {
                                        if let Some(ref value) = other_source.value {
                                            other_source.value = Some(media.adjust_msid_semantic(
                                                value.as_str(),
                                                line_type.to_string().as_str(),
                                            ))
                                        }
                                    }
                                }

                                if let Some(ref other) = other_source {
                                    new_line.add_attribute(SdpAttribute::Ssrc(
                                        SdpAttributeSsrc {
                                            id: other.id,
                                            attribute: other.attribute.clone(),
                                            value: other.value.clone(),
                                        },
                                    ))?;
                                }
                            }

                            new_line
                                .add_attribute(SdpAttribute::SsrcGroup(g.0.clone(), g.1.clone()))?;
                        } else {
                            new_line.add_attribute(SdpAttribute::Ssrc(SdpAttributeSsrc {
                                id: ssrc.id,
                                attribute: ssrc.attribute.clone(),
                                value: ssrc.value.clone(),
                            }))?;
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

                if !groups.is_empty() {
                    new_sdp.add_attribute(SdpAttribute::Group(SdpAttributeGroup {
                        semantics: SdpAttributeGroupSemantic::Bundle,
                        tags: mids,
                    }))?;
                }

                let msids: Vec<String> = new_media
                    .iter()
                    .flat_map(|media| media.get_attributes())
                    .filter_map(|attr| {
                        if let SdpAttribute::Ssrc(ssrc) = attr {
                            if ssrc.attribute.as_deref() == Some("msid") {
                                ssrc.value.as_ref().and_then(|v| {
                                    v.split_whitespace().next().map(|s| s.to_string())
                                })
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();

                new_sdp.add_attribute(SdpAttribute::MsidSemantic(SdpAttributeMsidSemantic {
                    semantic: "WMS".to_string(),
                    msids,
                }))?;

                Ok(new_sdp.to_string())
            }

            Err(err) => Err(Box::new(err)),
        }
    }
}

impl<'a> Display for JingleAction<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionInitiate(_) => write!(f, "session-initiate"),
            Self::SourceAdd(_) => write!(f, "source-add"),
        }
    }
}
