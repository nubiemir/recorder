#[macro_export]
macro_rules! make_stanza {
    ($name:expr, {
        $($key:expr => $value:expr),* $(,)?
    }) => {{
        (|| -> Result<::libstrophe::Stanza, ::libstrophe::Error> {
            let mut stanza = Stanza::new();
            stanza.set_name($name)?;
            $(stanza.set_attribute($key, $value)?;)*
                Ok(stanza)
        })()
    }};

    ($name:expr) => {{
        let mut stanza = Stanza::new()
            stanza.set_name($name);
    }};
}
