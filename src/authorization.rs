use shuttle_runtime::SecretStore;
use std::sync::Arc;

/// Dummy implementation for authorization
#[derive(Clone)]
pub struct Authorization {
    secret_store: Arc<SecretStore>,
}

/// RSS Readers do not allow providing headers, so we need to pass the token as a query parameter
/// alternatively, we could use a user:pass combination in the URI
/// DO NOT TRY THIS AT HOME
#[derive(serde::Deserialize)]
pub struct QueryToken {
    pub token: String,
}

impl Authorization {
    pub const fn new(secret_store: Arc<SecretStore>) -> Self {
        Self { secret_store }
    }

    pub fn authorize(&self, query_token: &QueryToken) -> bool {
        let token = self.secret_store.get("BASIC_TOKEN").unwrap();
        query_token.token == token
    }
}
