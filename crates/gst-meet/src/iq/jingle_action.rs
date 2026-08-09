//! Jingle actions, and the Jingle-to-SDP conversion behind `session-initiate`.

use std::fmt::Display;

use chrono::Utc;
use libstrophe::Stanza;
use log::{debug, error, info, warn};
use webrtc_sdp::{
    attribute_type::{
        SdpAttribute, SdpAttributeGroup, SdpAttributeGroupSemantic, SdpAttributeMsidSemantic,
        SdpAttributeSsrc, SdpAttributeType, SdpSsrcGroupSemantic,
    },
    media_type::{SdpMedia, SdpMediaValue},
    parse_sdp,
};

use crate::{
    iq::jingle_media::JingleMedia,
    util::{find_all, find_first},
};

/// A Jingle action the recorder acts on, holding the `<jingle>` stanza it came
/// from. Actions we don't implement never become a value of this type.
#[derive(Debug)]
pub enum JingleAction<'a> {
    SessionInitiate(&'a Stanza),
    SourceAdd(&'a Stanza),
    SourceRemove(&'a Stanza),
}

/// One stream announced by the bridge, resolved from Jitsi's JSON source
/// encoding.
pub struct ParsedSource {
    pub ssrc: u32,
    /// Endpoint that owns the stream.
    pub endpoint_id: String,
    /// Jitsi source name, `<endpoint>-v0` / `-a0` / `-v1`.
    pub source_name: String,
    pub video_type: Option<String>, // Some("d") => screenshare video; None for audio/camera
}

impl<'a> JingleAction<'a> {
    /// Recognizes an action name, or `None` for actions we ignore.
    pub(crate) fn parse(s: &str, stanza: &'a Stanza) -> Option<Self> {
        match s {
            "session-initiate" => Some(Self::SessionInitiate(stanza)),
            "source-add" => Some(Self::SourceAdd(stanza)),
            "source-remove" => Some(Self::SourceRemove(stanza)),
            _ => None,
        }
    }

    /// Translates a `session-initiate` into the SDP offer handed to
    /// `webrtcbin`, or an empty string if the result won't parse as SDP.
    ///
    /// Built in three passes: the session header, then one m-line per Jingle
    /// `<content>`, then [`final_parsing`](Self::final_parsing) to split and
    /// renumber m-lines the way the WebRTC stack expects.
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

    /// Extracts the SSRC-to-source mapping from a `source-add`.
    ///
    /// Sources arrive as a `<json-message>` keyed by endpoint, whose value is
    /// `[video, ssrc_groups, audio]`. Two kinds of entry are skipped: the
    /// bridge's own `jvb*` endpoint, and RTX retransmission SSRCs, which show
    /// up as the second member of an `"f"` (FID) group and carry no media of
    /// their own.
    ///
    /// Video is walked before audio so that a downstream sibling lookup finds
    /// the video source already registered at the same index.
    pub fn handle_source_add(&self, stanza: &Stanza) -> Vec<ParsedSource> {
        let mut res = vec![];

        let json_raw = match find_first(Some(&stanza), "json-message").and_then(|s| s.text()) {
            Some(text) => text,
            None => {
                error!("unable to find source add json message");
                return res;
            }
        };
        let json: serde_json::Value = match serde_json::from_str(&json_raw) {
            Ok(v) => v,
            Err(e) => {
                error!("source-add: failed to parse json: {e}");
                return res;
            }
        };
        let sources = match json["sources"].as_object() {
            Some(s) => s,
            None => {
                error!("source-add: no sources object");
                return res;
            }
        };

        for (endpoint_id, data) in sources {
            // RTX ssrcs from FID groups in data[1]; skip them.
            if endpoint_id.starts_with("jvb") {
                continue;
            };
            let empty = vec![];
            let rtx_ssrcs: std::collections::HashSet<u32> = data[1]
                .as_array()
                .unwrap_or(&empty)
                .iter()
                .filter_map(|g| g.as_array())
                .filter(|g| g.first().and_then(|v| v.as_str()) == Some("f"))
                .filter_map(|g| g.get(2).and_then(|v| v.as_u64()).map(|v| v as u32))
                .collect();

            // VIDEO (data[0]) before AUDIO (data[2]) so that downstream, audio's
            // sibling lookup finds the already-registered video at the same index.
            for arr_idx in [0usize, 2usize] {
                let media_sources = match data[arr_idx].as_array() {
                    Some(s) => s,
                    None => continue,
                };
                for source in media_sources {
                    let ssrc = match source["s"].as_u64() {
                        Some(s) => s as u32,
                        None => continue,
                    };
                    if rtx_ssrcs.contains(&ssrc) {
                        info!("source-add: skipping RTX ssrc={}", ssrc);
                        continue;
                    }
                    let source_name = source["n"].as_str().unwrap_or("").to_string();
                    let video_type = source["v"].as_str().map(|s| s.to_string());

                    info!(
                        "source-add: parsed ssrc={} endpoint={} name={} v={:?}",
                        ssrc, endpoint_id, source_name, video_type
                    );

                    res.push(ParsedSource {
                        ssrc,
                        endpoint_id: endpoint_id.to_string(),
                        source_name,
                        video_type,
                    });
                }
            }
        }

        res
    }

    /// Logs a `source-remove`. Not acted on: a removed source stops producing
    /// RTP, and the branch recording it is finalized on participant-left or at
    /// meeting end.
    pub fn handle_source_remove(&self, stanza: &Stanza) -> String {
        info!("source removed: {}", stanza.to_string());
        String::new()
    }

    /// Builds the SDP session header (`v=`, `o=`, `s=`, `t=`), adding
    /// `a=cryptex` when the bridge advertises it on any fingerprint.
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

    /// Reshapes the Jingle-derived SDP into an offer `webrtcbin` accepts.
    ///
    /// Jingle packs every SSRC for a media type onto one `<content>`, but
    /// WebRTC wants one m-line per stream (unified plan). So each m-line is
    /// split: one copy per primary SSRC, each with a fresh `mid`, carrying its
    /// own FID group and the RTX SSRC paired with it. The first copy stays
    /// `sendrecv` (it is the transceiver we answer on); the rest become
    /// `sendonly`.
    ///
    /// Finally it re-attaches a BUNDLE group over the new mids and rebuilds
    /// `a=msid-semantic` from the msids that survived.
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
            Self::SourceRemove(_) => write!(f, "source-remove"),
        }
    }
}
