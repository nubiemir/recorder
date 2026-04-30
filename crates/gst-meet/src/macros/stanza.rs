#[macro_export]
macro_rules! make_stanza {
    ($name:expr, {
        $($key:expr => $value:expr),* $(,)?
    },[
    $($child:expr),* $(,)?
    ]) => {{
        (|| -> Result<::libstrophe::Stanza, ::libstrophe::Error> {
            let mut stanza = ::libstrophe::Stanza::new();

            stanza.set_name($name)?;

            $(
                stanza.set_attribute($key, $value)?;
            )*

                $(
                    stanza.add_child($child)?;
                )*

                Ok(stanza)
        })()
    }};

    ($name:expr, [
     $($child:expr),* $(,)?
    ]) => {{

        (|| -> Result<::libstrophe::Stanza, ::libstrophe::Error> {
            let mut stanza = Stanza::new()
                stanza.set_name($name);

            $(
                stanza.add_child($child)?;
            )*
        })()
    }};
}

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
