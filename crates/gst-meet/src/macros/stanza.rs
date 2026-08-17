//! Declarative construction and reading of libstrophe stanzas.

/// Builds a [`Stanza`](libstrophe::Stanza) from a name, optional attributes and
/// optional children, evaluating to `Result<Stanza, libstrophe::Error>`.
///
/// ```ignore
/// let iq = make_stanza!("iq", { "type" => "set", "to" => jid }, [jingle])?;
/// let ping = make_stanza!("ping", { "xmlns" => XEP::Jingle.to_string() })?;
/// let wrap = make_stanza!("content", [description, transport])?;
/// ```
///
/// Every form takes an optional trailing `text: <expr>` giving the element's
/// text content, which is emitted before any children:
///
/// ```ignore
/// let stats = make_stanza!("stats-id", {}, text: id)?;   // <stats-id>id</stats-id>
/// ```
///
/// Text cannot be set on the element stanza itself: libstrophe stanzas are
/// either tag nodes or text nodes, and `set_text` on one that already has a
/// name fails with `XMPP_EINVOP`. So the text becomes an unnamed child stanza,
/// which is also how it is represented in XML.
///
/// The body expands inside an immediately-invoked closure so `?` short-circuits
/// on the first failing `set_attribute` / `add_child`.
#[macro_export]
macro_rules! make_stanza {
    ($name:expr, {
        $($key:expr => $value:expr),* $(,)?
    },[
    $($child:expr),* $(,)?
    ] $(, text: $text:expr)?) => {{
        (|| -> Result<::libstrophe::Stanza, ::libstrophe::Error> {
            let mut stanza = ::libstrophe::Stanza::new();

            stanza.set_name($name)?;

            $(
                stanza.set_attribute($key, $value)?;
            )*

                $(
                    let mut text_node = ::libstrophe::Stanza::new();
                    text_node.set_text($text)?;
                    stanza.add_child(text_node)?;
                )?

                $(
                    stanza.add_child($child)?;
                )*

                Ok(stanza)
        })()
    }};


    ($name:expr, {
        $($key:expr => $value:expr),* $(,)?
    } $(, text: $text:expr)?) => {{
        (|| -> Result<::libstrophe::Stanza, ::libstrophe::Error> {
            let mut stanza = ::libstrophe::Stanza::new();

            stanza.set_name($name)?;

            $(
                stanza.set_attribute($key, $value)?;
            )*

                $(
                    let mut text_node = ::libstrophe::Stanza::new();
                    text_node.set_text($text)?;
                    stanza.add_child(text_node)?;
                )?

                Ok(stanza)
        })()
    }};



    ($name:expr, [
     $($child:expr),* $(,)?
    ] $(, text: $text:expr)?) => {{

        (|| -> Result<::libstrophe::Stanza, ::libstrophe::Error> {
            let mut stanza = ::libstrophe::Stanza::new();
            stanza.set_name($name)?;

            $(
                let mut text_node = ::libstrophe::Stanza::new();
                text_node.set_text($text)?;
                stanza.add_child(text_node)?;
            )?

            $(
                stanza.add_child($child)?;
            )*


                Ok(stanza)
        })()
    }};
}

/// Sets several attributes on an existing stanza, evaluating to
/// `Result<(), libstrophe::Error>`.
///
/// ```ignore
/// set_attribute!(stanza, { "xmlns" => ns, "senders" => "both" })?;
/// ```
#[macro_export]
macro_rules! set_attribute {
    ($stanza:expr, {
        $($key:expr => $value:expr),* $(,)?
    }) => {
        (|| -> Result<(), ::libstrophe::Error> {

            $(
                $stanza.set_attribute($key, $value)?;
            )*


                Ok(())
        })()

    };
}

/// Reads attributes off a stanza into an ad-hoc struct with one `String` field
/// per attribute; missing attributes come back empty rather than erroring.
///
/// ```ignore
/// let fields = get_attribute!(stanza, [from, to, id]);       // field == attribute
/// let fields = get_attribute!(stanza, { sid => "sid",        // field != attribute
///                                       initiator => "initiator" });
/// ```
///
/// The struct type is declared inside the expansion, so it exists only for the
/// expression it is used in.
#[macro_export]
macro_rules! get_attribute{
    ($stanza:expr, [$($field:ident),+]) => {{
        #[derive(Debug)]
        #[allow(unused)]
        struct StanzaFields {
            $($field: String),+
        }

        StanzaFields {
            $($field: $stanza.get_attribute(stringify!($field))
                .unwrap_or_default()
                .to_string()
            ),+
        }
    }};
    ($stanza:expr, {$($field:ident => $attr:expr),+}) => {{
        #[derive(Debug)]
        #[allow(unused)]
        struct StanzaFields {
            $($field: String),+
        }

        StanzaFields {
            $($field: $stanza.get_attribute($attr)
                .unwrap_or_default()
                .to_string()
            ),+
        }
    }};
}
