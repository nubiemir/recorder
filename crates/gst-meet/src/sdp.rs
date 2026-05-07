use libstrophe::{Error, Stanza};
use nanoid::nanoid;
use webrtc_sdp::{
    SdpSession,
    address::Address,
    attribute_type::{
        SdpAttribute, SdpAttributeCandidate, SdpAttributeFingerprint, SdpAttributeFmtp,
        SdpAttributeFmtpParameters, SdpAttributeRtcpFb, SdpAttributeRtcpFbType, SdpAttributeRtpmap,
        SdpAttributeType,
    },
    media_type::{SdpFormatList, SdpMedia},
};

use crate::{make_stanza, set_attribute, xep::XEP};

pub struct Sdp<'a>(&'a SdpSession);

impl<'a> Sdp<'a> {
    pub fn new(sdp_session: &'a SdpSession) -> Self {
        Sdp(sdp_session)
    }

    pub fn parse_sdp_to_jingle(
        &self,
        initiator: &str,
        sid: &str,
        responder: &str,
    ) -> Result<Stanza, Error> {
        let mut jingle_stanza = make_stanza!("jingle", {
            "xmlns" => "urn:xmpp:jingle:1",
            "action" => "session-accept",
            "initiator" => initiator,
            "sid" => sid,
            "responder" => responder,
        })?;

        self.parse_group(&mut jingle_stanza)?;

        self.parse_media(&mut jingle_stanza)?;

        Ok(jingle_stanza)
    }

    fn parse_group(&self, stanza: &mut Stanza) -> Result<(), Error> {
        let sdp_group = self.0.get_attribute(SdpAttributeType::Group);
        if let Some(group) = sdp_group {
            if let SdpAttribute::Group(group_attr) = group {
                let mut group_stanza = make_stanza!("group", {
                    "semantics" => group_attr.semantics.to_string(),
                    "xmlns" => XEP::BundleMedia.to_string(),
                })?;
                for content in ["audio", "video"] {
                    let content_stanza = make_stanza!("content", {
                        "name" => content.to_string(),
                    })?;

                    group_stanza.add_child(content_stanza)?;
                }
                stanza.add_child(group_stanza)?;
            }
        }
        Ok(())
    }

    fn parse_media(&self, jingle_stanza: &mut Stanza) -> Result<(), Error> {
        for media in self.0.media.iter() {
            let media_type = media.get_type().to_string();
            let mut content_stanza = make_stanza!("content", {
                "name" => media_type.to_string(),
                "creator" =>  "initiator",
                "senders" => "initiator"
            })?;

            let mut description_stanza = make_stanza!("description", {})?;

            if ["audio", "video"].contains(&media_type.as_str()) {
                set_attribute!(description_stanza, {
                "media" => media_type,
                "xmlns" => XEP::RtpMedia.to_string()
                })?;

                match media.get_formats() {
                    //TODO: check out this comment letter on
                    // SdpFormatList::Strings(formats) => for format in formats {},
                    SdpFormatList::Integers(formats) => {
                        for format in formats {
                            let rtpmap = self.find_rtpmap(media, format.clone());

                            if let Some(rtpmap) = rtpmap {
                                let mut payload_type_stanza = make_stanza!("payload-type", {
                                    "id" => rtpmap.payload_type.to_string(),
                                    "clockrate" => rtpmap.frequency.to_string(),
                                    "channels" => rtpmap.channels.unwrap_or(1).to_string(),
                                    "name" => rtpmap.codec_name.to_string()
                                })?;

                                let fmtp = self.find_fmtp(media, format.clone());

                                if let Some(fmtp) = fmtp {
                                    for param in self.fmtp_to_params(&fmtp.parameters) {
                                        let parameter_stanza = make_stanza!("parameter", {
                                            param.0 => param.1,
                                        })?;
                                        payload_type_stanza.add_child(parameter_stanza)?;
                                    }
                                }

                                self.rtcp_fb_to_jingle(media, &mut payload_type_stanza)?;

                                description_stanza.add_child(payload_type_stanza)?;
                            }
                        }
                    }
                    _ => {}
                }

                if self.rtcp_mux_exist(media) {
                    let rtcp_mux_stanza = make_stanza!("rtcp-mux", {})?;
                    description_stanza.add_child(rtcp_mux_stanza)?;
                }

                let mut transport_stanza = make_stanza!("transport", {})?;
                self.transport_to_jingle(media, &mut transport_stanza)?;

                content_stanza.add_child(description_stanza)?;
                content_stanza.add_child(transport_stanza)?;
            }
            jingle_stanza.add_child(content_stanza)?;
        }

        Ok(())
    }

    fn rtcp_mux_exist(&self, media: &SdpMedia) -> bool {
        let rtcp_mux = media
            .get_attributes()
            .iter()
            .any(|rtcp_mux| match rtcp_mux {
                SdpAttribute::RtcpMux => true,
                _ => false,
            });

        rtcp_mux
    }

    fn find_rtpmap(&self, media: &'a SdpMedia, format: u32) -> Option<&'a SdpAttributeRtpmap> {
        let rtpmap = media.get_attributes().iter().find_map(|attr| match attr {
            SdpAttribute::Rtpmap(rtpmap) if rtpmap.payload_type as u32 == format => Some(rtpmap),
            _ => None,
        });
        rtpmap
    }

    fn find_fmtp(&self, media: &'a SdpMedia, format: u32) -> Option<&'a SdpAttributeFmtp> {
        let fmtp = media.get_attributes().iter().find_map(|attr| match attr {
            SdpAttribute::Fmtp(fmtp) if fmtp.payload_type as u32 == format => Some(fmtp),
            _ => None,
        });
        fmtp
    }

    fn fmtp_to_params(&self, fmtp: &SdpAttributeFmtpParameters) -> Vec<(String, String)> {
        fn b(v: bool) -> &'static str {
            if v { "1" } else { "0" }
        }

        let mut params = Vec::new();

        // Integer fields
        if fmtp.packetization_mode != 0 {
            params.push((
                "packetization-mode".into(),
                fmtp.packetization_mode.to_string(),
            ));
        }

        if fmtp.profile_level_id != 0 {
            params.push((
                "profile-level-id".into(),
                format!("{:06x}", fmtp.profile_level_id),
            ));
        }

        if fmtp.max_fs != 0 {
            params.push(("max-fs".into(), fmtp.max_fs.to_string()));
        }

        if fmtp.max_cpb != 0 {
            params.push(("max-cpb".into(), fmtp.max_cpb.to_string()));
        }

        if fmtp.max_dpb != 0 {
            params.push(("max-dpb".into(), fmtp.max_dpb.to_string()));
        }

        if fmtp.max_br != 0 {
            params.push(("max-br".into(), fmtp.max_br.to_string()));
        }

        if fmtp.max_mbps != 0 {
            params.push(("max-mbps".into(), fmtp.max_mbps.to_string()));
        }

        if fmtp.max_fr != 0 {
            params.push(("max-fr".into(), fmtp.max_fr.to_string()));
        }

        if fmtp.maxplaybackrate != 0 {
            params.push(("maxplaybackrate".into(), fmtp.maxplaybackrate.to_string()));
        }

        if fmtp.maxaveragebitrate != 0 {
            params.push((
                "maxaveragebitrate".into(),
                fmtp.maxaveragebitrate.to_string(),
            ));
        }

        if fmtp.ptime != 0 {
            params.push(("ptime".into(), fmtp.ptime.to_string()));
        }

        if fmtp.minptime != 0 {
            params.push(("minptime".into(), fmtp.minptime.to_string()));
        }

        if fmtp.maxptime != 0 {
            params.push(("maxptime".into(), fmtp.maxptime.to_string()));
        }

        // Boolean fields (always included as requested)
        params.push((
            "level-asymmetry-allowed".into(),
            b(fmtp.level_asymmetry_allowed).into(),
        ));
        params.push(("usedtx".into(), b(fmtp.usedtx).into()));
        params.push(("stereo".into(), b(fmtp.stereo).into()));
        params.push(("useinbandfec".into(), b(fmtp.useinbandfec).into()));
        params.push(("cbr".into(), b(fmtp.cbr).into()));

        // Optional numeric fields
        if let Some(profile) = fmtp.profile {
            params.push(("profile".into(), profile.to_string()));
        }

        if let Some(level_idx) = fmtp.level_idx {
            params.push(("level-idx".into(), level_idx.to_string()));
        }

        if let Some(tier) = fmtp.tier {
            params.push(("tier".into(), tier.to_string()));
        }

        // Vec fields
        if !fmtp.encodings.is_empty() {
            let encodings = fmtp
                .encodings
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(",");
            params.push(("encodings".into(), encodings));
        }

        // String field
        if !fmtp.dtmf_tones.is_empty() {
            params.push(("dtmf-tones".into(), fmtp.dtmf_tones.clone()));
        }

        // RTX
        if let Some(rtx) = &fmtp.rtx {
            params.push(("apt".into(), rtx.apt.to_string()));
        }

        // Unknown tokens (already "key=value")
        for token in &fmtp.unknown_tokens {
            if let Some((k, v)) = token.split_once('=') {
                params.push((k.to_string(), v.to_string()));
            }
        }

        params
    }

    fn rtcp_fb_to_jingle(&self, media: &SdpMedia, stanza: &mut Stanza) -> Result<(), Error> {
        let rtcp_fbs: Vec<SdpAttributeRtcpFb> = media
            .get_attributes_of_type(SdpAttributeType::Rtcpfb)
            .iter()
            .filter_map(|a| {
                if let SdpAttribute::Rtcpfb(s) = a {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect();

        for rtcp_fb in rtcp_fbs {
            match rtcp_fb.feedback_type {
                SdpAttributeRtcpFbType::TrrInt => {
                    let rtcp_fb_stanza = make_stanza!("rtcp-fb-trr-int", {
                        "value" => rtcp_fb.parameter,
                        "xmlns" => XEP::RtpFeedback.to_string(),
                    })?;
                    stanza.add_child(rtcp_fb_stanza)?;
                }

                _ => {
                    let rtcp_fb_stanza = make_stanza!("rtcp_fb", {
                        "xmlns" => XEP::RtpFeedback.to_string(),
                        "type" => rtcp_fb.feedback_type.to_string(),
                        "subtype" => rtcp_fb.parameter
                    })?;
                    stanza.add_child(rtcp_fb_stanza)?;
                }
            }
        }

        Ok(())
    }
    fn transport_to_jingle(&self, media: &SdpMedia, stanza: &mut Stanza) -> Result<(), Error> {
        let fingerprints: Vec<SdpAttributeFingerprint> = media
            .get_attributes_of_type(SdpAttributeType::Fingerprint)
            .iter()
            .filter_map(|attr| {
                if let SdpAttribute::Fingerprint(fp) = attr {
                    Some(fp.clone())
                } else {
                    None
                }
            })
            .collect();

        let setup = media.get_attribute(SdpAttributeType::Setup);

        for fingerprint in fingerprints {
            let mut fingerprint_stanza = make_stanza!("fingerprint", {
                "xmlns" => XEP::DtlsSrtp.to_string(),
                "hash" => fingerprint.hash_algorithm.to_string(),
            })?;

            let text = self.fingerprint_to_hex(&fingerprint.fingerprint);
            let mut fp_text = Stanza::new();
            fp_text.set_text(text)?;
            fingerprint_stanza.add_child(fp_text)?;

            if let Some(SdpAttribute::Setup(setup)) = setup {
                set_attribute!(fingerprint_stanza, {
                    "setup" => setup.to_string(),
                })?;
            }

            stanza.add_child(fingerprint_stanza)?;
        }

        let ice_ufrag = media.get_attribute(SdpAttributeType::IceUfrag);
        let ice_pwd = media.get_attribute(SdpAttributeType::IcePwd);

        if let (Some(SdpAttribute::IceUfrag(ufrag)), Some(SdpAttribute::IcePwd(pwd))) =
            (ice_ufrag, ice_pwd)
        {
            set_attribute!(stanza, {
                "ufrag" => ufrag,
                "pwd" => pwd,
                "xmlns" => XEP::IceUdpTransport.to_string()
            })?;

            let ice_candidates: Vec<SdpAttributeCandidate> = media
                .get_attributes_of_type(SdpAttributeType::Candidate)
                .iter()
                .filter_map(|attr| {
                    if let SdpAttribute::Candidate(candidate) = attr {
                        Some(candidate.clone())
                    } else {
                        None
                    }
                })
                .collect();

            for candidate in ice_candidates {
                let mut candidate_stanza = make_stanza!("candidate", {})?;
                if let Ok(()) = self.parse_candidate(&mut candidate_stanza, &candidate) {
                    stanza.add_child(candidate_stanza)?;
                }
            }
        }

        Ok(())
    }

    fn fingerprint_to_hex(&self, fp: &[u8]) -> String {
        let hx = fp
            .iter()
            .map(|b| format!("{:02x}", b).to_uppercase())
            .collect::<Vec<_>>()
            .join(":");

        hx
    }

    fn parse_candidate(
        &self,
        candidate_stanza: &mut Stanza,
        candidate: &SdpAttributeCandidate,
    ) -> Result<(), Error> {
        set_attribute!(candidate_stanza, {
            "port" => candidate.port.to_string(),
            "component" => candidate.component.to_string(),
            "foundation" => candidate.foundation.to_string(),
            "type" => candidate.c_type.to_string()
        })?;

        match candidate.address.clone() {
            Address::Ip(ip) => {
                set_attribute!(candidate_stanza, {
                    "ip" => ip.to_string(),
                })?;
            }
            Address::Fqdn(fqdn) => {
                set_attribute!(candidate_stanza, {
                    "ip" => fqdn.to_string(),
                })?;
            }
        }
        set_attribute!(candidate_stanza, {
            "priority" => candidate.priority.to_string(),
        })?;

        if let Some(raddr) = candidate.raddr.clone() {
            match raddr {
                Address::Ip(ip) => {
                    set_attribute!(candidate_stanza, {
                        "rel-addr" => ip.to_string(),
                    })?;
                }

                Address::Fqdn(fqdn) => {
                    set_attribute!(candidate_stanza, {
                        "rel-addr" => fqdn.to_string(),
                    })?;
                }
            }
        }

        if let Some(rport) = candidate.rport {
            set_attribute!(candidate_stanza, {
                "rel-port" => rport.to_string(),
            })?;
        }

        set_attribute!(candidate_stanza, {
            "generation" => candidate.generation.unwrap_or_default().to_string(),
            "network" => "1",
            "id" => nanoid!(),
            "protocol" => candidate.transport.to_string().to_lowercase()
        })?;

        Ok(())
    }
}
