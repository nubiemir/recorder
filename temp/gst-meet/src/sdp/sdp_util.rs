use libstrophe::{Error, Stanza};
use nanoid::nanoid;
use webrtc_sdp::{
    address::Address,
    attribute_type::{
        SdpAttribute, SdpAttributeCandidate, SdpAttributeFingerprint, SdpAttributeFmtp,
        SdpAttributeFmtpParameters, SdpAttributeRtcpFb, SdpAttributeRtcpFbType, SdpAttributeRtpmap,
        SdpAttributeType,
    },
    media_type::SdpMedia,
};

use crate::xmpp::xep::XEP;

pub fn find_fmtp(media: &SdpMedia, format: u32) -> Option<&SdpAttributeFmtp> {
    let fmtp = media.get_attributes().iter().find_map(|attr| match attr {
        SdpAttribute::Fmtp(fmtp) if fmtp.payload_type as u32 == format => Some(fmtp),
        _ => None,
    });
    fmtp
}

pub fn find_rtpmap(media: &SdpMedia, format: u32) -> Option<&SdpAttributeRtpmap> {
    let rtpmap = media.get_attributes().iter().find_map(|attr| match attr {
        SdpAttribute::Rtpmap(rtpmap) if rtpmap.payload_type as u32 == format => Some(rtpmap),
        _ => None,
    });
    rtpmap
}

pub fn rtcp_mux_exist(media: &SdpMedia) -> bool {
    let rtcp_mux = media
        .get_attributes()
        .iter()
        .any(|rtcp_mux| match rtcp_mux {
            SdpAttribute::RtcpMux => true,
            _ => false,
        });

    rtcp_mux
}

pub fn fmtp_to_params(fmtp: &SdpAttributeFmtpParameters) -> Vec<(String, String)> {
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

pub fn rtcp_fb_to_jingle(media: &SdpMedia, stanza: &mut Stanza) -> Result<(), Error> {
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
        let mut rtcp_fb_stanza = Stanza::new();
        match rtcp_fb.feedback_type {
            SdpAttributeRtcpFbType::TrrInt => {
                rtcp_fb_stanza.set_name("rtcp-fb-trr-int")?;
                rtcp_fb_stanza.set_attribute("value", rtcp_fb.parameter)?;
                rtcp_fb_stanza.set_ns(XEP::RtpFeedback.to_string())?;
            }

            _ => {
                rtcp_fb_stanza.set_name("rtcp-fp")?;
                rtcp_fb_stanza.set_ns(XEP::RtpFeedback.to_string())?;
                rtcp_fb_stanza.set_attribute("type", rtcp_fb.feedback_type.to_string())?;
                rtcp_fb_stanza.set_attribute("subtype", rtcp_fb.parameter)?;
            }
        }
        stanza.add_child(rtcp_fb_stanza)?;
    }

    Ok(())
}

fn fingerprint_to_hex(fp: &[u8]) -> String {
    let hx = fp
        .iter()
        .map(|b| format!("{:02x}", b).to_uppercase())
        .collect::<Vec<_>>()
        .join(":");

    hx
}

pub fn parse_candidate(
    candidate_stanza: &mut Stanza,
    candidate: &SdpAttributeCandidate,
) -> Result<(), Error> {
    candidate_stanza.set_name("candidate")?;
    candidate_stanza.set_attribute("port", candidate.port.to_string())?;
    candidate_stanza.set_attribute("component", candidate.component.to_string())?;
    candidate_stanza.set_attribute("foundation", candidate.foundation.to_string())?;
    candidate_stanza.set_attribute("type", candidate.c_type.to_string())?;
    match candidate.address.clone() {
        Address::Ip(ip) => {
            candidate_stanza.set_attribute("ip", ip.to_string())?;
        }
        Address::Fqdn(fqdn) => {
            candidate_stanza.set_attribute("ip", fqdn)?;
        }
    }
    candidate_stanza.set_attribute("priority", candidate.priority.to_string())?;

    if let Some(raddr) = candidate.raddr.clone() {
        match raddr {
            Address::Ip(ip) => {
                candidate_stanza.set_attribute("rel-addr", ip.to_string())?;
            }

            Address::Fqdn(fqdn) => {
                candidate_stanza.set_attribute("rel-addr", fqdn)?;
            }
        }
    }

    if let Some(rport) = candidate.rport {
        candidate_stanza.set_attribute("rel-port", rport.to_string())?;
    }

    candidate_stanza.set_attribute(
        "generation",
        candidate.generation.unwrap_or_default().to_string(),
    )?;

    candidate_stanza.set_attribute("network", "1")?;
    candidate_stanza.set_attribute("id", nanoid!())?;
    candidate_stanza.set_attribute("protocol", candidate.transport.to_string().to_lowercase())?;
    Ok(())
}

pub fn transport_to_jingle(media: &SdpMedia, stanza: &mut Stanza) -> Result<(), Error> {
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
        let mut fingerprint_stanza = Stanza::new();
        fingerprint_stanza.set_name("fingerprint")?;
        fingerprint_stanza.set_ns(XEP::DtlsSrtp.to_string())?;
        fingerprint_stanza.set_attribute("hash", fingerprint.hash_algorithm.to_string())?;

        let text = fingerprint_to_hex(&fingerprint.fingerprint);
        let mut fp_text = Stanza::new();
        fp_text.set_text(text)?;
        fingerprint_stanza.add_child(fp_text)?;

        if let Some(SdpAttribute::Setup(setup)) = setup {
            fingerprint_stanza.set_attribute("setup", setup.to_string())?;
        }

        stanza.add_child(fingerprint_stanza)?;
    }

    let ice_ufrag = media.get_attribute(SdpAttributeType::IceUfrag);
    let ice_pwd = media.get_attribute(SdpAttributeType::IcePwd);

    if let (Some(SdpAttribute::IceUfrag(ufrag)), Some(SdpAttribute::IcePwd(pwd))) =
        (ice_ufrag, ice_pwd)
    {
        stanza.set_attribute("ufrag", ufrag)?;
        stanza.set_attribute("pwd", pwd)?;
        stanza.set_ns(XEP::IceUdpTransport.to_string())?;

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
            let mut candidate_stanza = Stanza::new();
            if let Ok(()) = parse_candidate(&mut candidate_stanza, &candidate) {
                stanza.add_child(candidate_stanza)?;
            }
        }
    }

    Ok(())
}
