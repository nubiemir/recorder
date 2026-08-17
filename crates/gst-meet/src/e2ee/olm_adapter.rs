use std::fmt;

use vodozemac::olm::{Account, IdentityKeys};

/// The recorder's Olm identity for E2EE key exchange.
///
/// The account holds the private Ed25519 signing key and Curve25519 identity
/// key in memory only — vodozemac zeroizes them on drop and nothing here
/// pickles them, so each [`OlmAdapter::new`] is a fresh identity that does not
/// survive a restart. Persisting one would mean pickling the account and
/// protecting the pickle key.
pub struct OlmAdapter {
    olm_account: Account,
}

impl fmt::Debug for OlmAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OlmAdapter")
            .field("olm_account", &"<redacted>")
            .finish()
    }
}

impl OlmAdapter {
    /// Generates a new, ephemeral identity.
    pub fn new() -> Self {
        let account = Account::new();
        let olm_adapter = Self {
            olm_account: account,
        };
        olm_adapter.on_idkeys_ready();

        olm_adapter
    }

    /// The *public* identity keys, which are what gets published to other
    /// participants for verification.
    pub fn get_id_keys(&self) -> IdentityKeys {
        self.olm_account.identity_keys()
    }

    fn on_idkeys_ready(&self) {}
}
