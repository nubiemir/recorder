use libstrophe::Stanza;

pub fn find_all(element: Option<&Stanza>, selector: &str) -> Vec<Stanza> {
    let mut result = vec![];
    let selector: Vec<&str> = selector.split(">").collect();
    if let Some(ele) = element {
        for c in ele.children() {
            let ele = c.get_child_by_path(&selector);
            match ele {
                Some(stan) => {
                    result.push(stan.clone());
                }
                None => {
                    continue;
                }
            }
        }
    }
    result
}

pub fn find_first(element: Option<&Stanza>, selector: &str) -> Option<Stanza> {
    let selector: Vec<&str> = selector.split(">").collect();

    if let Some(ele) = element {
        for c in ele.children() {
            if let Some(stan) = c.get_child_by_path(&selector) {
                return Some(stan.clone());
            }
        }
        None
    } else {
        None
    }
}

pub fn exists(element: Option<&Stanza>, selector: &str) -> bool {
    find_all(element, selector).len() != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use libstrophe::Stanza;

    #[test]
    fn test_find_transport() {
        let _ = env_logger::builder().is_test(true).try_init();
        let xml = r#"
        <iq>
           <jingle initiator="focus@auth.meet.jitsi/focus"
        xmlns="urn:xmpp:jingle:1" sid="6lrpse6evqv2c" action="session-initiate">
        <content name="audio" senders="both" creator="initiator">
            <description maxptime="60"
                xmlns="urn:xmpp:jingle:apps:rtp:1" media="audio">
                <payload-type id="111" clockrate="48000" channels="2" name="opus">
                    <parameter value="10" name="minptime"/>
                    <parameter value="1" name="useinbandfec"/>
                </payload-type>
                <payload-type id="126" clockrate="8000" name="telephone-event"/>
                <rtp-hdrext id="1"
                    xmlns="urn:xmpp:jingle:apps:rtp:rtp-hdrext:0" uri="urn:ietf:params:rtp-hdrext:ssrc-audio-level"/>
                <extmap-allow-mixed xmlns="urn:xmpp:jingle:apps:rtp:rtp-hdrext:0"/>
                <rtcp-mux/>
                <source xmlns="urn:xmpp:jingle:apps:rtp:ssma:0" ssrc="4024075403" name="jvb-a0">
                    <ssrc-info xmlns="http://jitsi.org/jitmeet" owner="jvb"/>
                    <parameter xmlns="urn:xmpp:jingle:apps:rtp:1" value="mixedmslabel mixedlabelaudio0" name="msid"/>
                </source>
            </description>
            <transport pwd="63hgqlcb8dusq35vo1j1gpjes" ufrag="f12or1jkjcuh77"
                xmlns="urn:xmpp:jingle:transports:ice-udp:1">
                <rtcp-mux/>
                <fingerprint hash="sha-256"
                    xmlns="urn:xmpp:jingle:apps:dtls:0" required="false" setup="actpass">2F:2F:4D:3A:76:ED:0B:8E:49:7D:8D:EF:D4:9B:24:7A:05:2D:29:35:70:EF:4D:04:C0:B4:70:59:54:50:F9:36</fingerprint>
                <candidate port="10000" component="1" id="a0f3c5d5448cdc80ffffffffb6ff1049" ip="172.18.0.4" network="0" priority="2130706431" generation="0" type="host" foundation="1" protocol="udp"/>
                <candidate port="10000" component="1" ip="5.31.242.40" id="138213895448cdc80100d026d" network="0" priority="1694498815" generation="0" rel-port="10000" rel-addr="172.18.0.4" type="srflx" foundation="2" protocol="udp"/>
            </transport>
        </content>
        <content name="video" senders="both" creator="initiator">
            <description xmlns="urn:xmpp:jingle:apps:rtp:1" media="video">
                <payload-type id="41" clockrate="90000" name="AV1">
                    <rtcp-fb subtype="fir"
                        xmlns="urn:xmpp:jingle:apps:rtp:rtcp-fb:0" type="ccm"/>
                    <rtcp-fb xmlns="urn:xmpp:jingle:apps:rtp:rtcp-fb:0" type="nack"/>
                    <rtcp-fb subtype="pli"
                        xmlns="urn:xmpp:jingle:apps:rtp:rtcp-fb:0" type="nack"/>
                </payload-type>
                <payload-type id="100" clockrate="90000" name="VP8">
                    <rtcp-fb subtype="fir"
                        xmlns="urn:xmpp:jingle:apps:rtp:rtcp-fb:0" type="ccm"/>
                    <rtcp-fb xmlns="urn:xmpp:jingle:apps:rtp:rtcp-fb:0" type="nack"/>
                    <rtcp-fb subtype="pli"
                        xmlns="urn:xmpp:jingle:apps:rtp:rtcp-fb:0" type="nack"/>
                </payload-type>
                <payload-type id="107" clockrate="90000" name="H264">
                    <rtcp-fb subtype="fir"
                        xmlns="urn:xmpp:jingle:apps:rtp:rtcp-fb:0" type="ccm"/>
                    <rtcp-fb xmlns="urn:xmpp:jingle:apps:rtp:rtcp-fb:0" type="nack"/>
                    <rtcp-fb subtype="pli"
                        xmlns="urn:xmpp:jingle:apps:rtp:rtcp-fb:0" type="nack"/>
                    <parameter value="42e01f;level-asymmetry-allowed=1;packetization-mode=1;" name="profile-level-id"/>
                </payload-type>
                <payload-type id="101" clockrate="90000" name="VP9">
                    <rtcp-fb subtype="fir"
                        xmlns="urn:xmpp:jingle:apps:rtp:rtcp-fb:0" type="ccm"/>
                    <rtcp-fb xmlns="urn:xmpp:jingle:apps:rtp:rtcp-fb:0" type="nack"/>
                    <rtcp-fb subtype="pli"
                        xmlns="urn:xmpp:jingle:apps:rtp:rtcp-fb:0" type="nack"/>
                </payload-type>
                <rtp-hdrext id="11"
                    xmlns="urn:xmpp:jingle:apps:rtp:rtp-hdrext:0" uri="https://aomediacodec.github.io/av1-rtp-spec/#dependency-descriptor-rtp-header-extension"/>
                <rtp-hdrext id="3"
                    xmlns="urn:xmpp:jingle:apps:rtp:rtp-hdrext:0" uri="http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time"/>
                <extmap-allow-mixed xmlns="urn:xmpp:jingle:apps:rtp:rtp-hdrext:0"/>
                <rtcp-mux/>
                <source xmlns="urn:xmpp:jingle:apps:rtp:ssma:0" ssrc="913855601" name="jvb-v0">
                    <ssrc-info xmlns="http://jitsi.org/jitmeet" owner="jvb"/>
                    <parameter xmlns="urn:xmpp:jingle:apps:rtp:1" value="mixedmslabel mixedlabelvideo0" name="msid"/>
                </source>
            </description>
            <transport pwd="63hgqlcb8dusq35vo1j1gpjes" ufrag="f12or1jkjcuh77"
                xmlns="urn:xmpp:jingle:transports:ice-udp:1">
                <rtcp-mux/>
                <fingerprint hash="sha-256"
                    xmlns="urn:xmpp:jingle:apps:dtls:0" required="false" setup="actpass">2F:2F:4D:3A:76:ED:0B:8E:49:7D:8D:EF:D4:9B:24:7A:05:2D:29:35:70:EF:4D:04:C0:B4:70:59:54:50:F9:36</fingerprint>
                <candidate port="10000" component="1" id="a0f3c5d5448cdc80ffffffffb6ff1049" ip="172.18.0.4" network="0" priority="2130706431" generation="0" type="host" foundation="1" protocol="udp"/>
                <candidate port="10000" component="1" ip="5.31.242.40" id="138213895448cdc80100d026d" network="0" priority="1694498815" generation="0" rel-port="10000" rel-addr="172.18.0.4" type="srflx" foundation="2" protocol="udp"/>
            </transport>
        </content>
        <group xmlns="urn:xmpp:jingle:apps:grouping:0" semantics="BUNDLE">
            <content name="audio"/>
            <content name="video"/>
        </group>
        <bridge-session id="14698ed3-2af7-4848-a942-d551eba727ab"
            xmlns="http://jitsi.org/protocol/focus"/>
    </jingle>        </iq>
        "#;

        // Parse XML into Stanza
        let iq = Stanza::from_str(xml);

        let jingle = iq.get_child_by_name("jingle").expect("no jingle");

        // let results = from_jingle(jingle.clone());
        //
        // println!("found: {}", results.len());
        //
        // // basic assertion
        // assert_eq!(results.len(), 2);
    }
}
