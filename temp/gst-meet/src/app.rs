use std::sync::Arc;

use libstrophe::Connection;

pub struct App<'a, 'b> {
    pub xmpp_connection: Arc<Connection<'a, 'b>>,
}

impl<'a, 'b> App<'a, 'b> {
    pub fn new(connection: Connection<'a, 'b>) -> Self {
        App {
            xmpp_connection: Arc::new(connection),
        }
    }
}
