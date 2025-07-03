use async_trait::async_trait;
use azure_core::http::headers::query_param;
use std::sync::Arc;

use typespec_client_core::http::{
    policies::{Policy, PolicyResult},
    Context, Request,
};

/// Creates a new SAS token policy for authentication.
///
/// # Arguments
///
/// * `token` - The SAS token to use for authentication
///
/// # Returns
///
/// Returns an Arc-wrapped Policy implementation that adds the SAS token to requests
pub(crate) fn create_sas_token_policy(token: String) -> Arc<dyn Policy> {
    Arc::new(SasTokenCredentialPolicy::new(token))
}

#[derive(Debug, Clone)]
struct SasTokenCredentialPolicy {
    token: String,
}

impl SasTokenCredentialPolicy {
    /// Creates a new `SasTokenCredentialPolicy` with the provided SAS token.
    fn new(token: String) -> Self {
        Self { token }
    }

    /// Returns the SAS token.
    pub fn token(&self) -> &str {
        &self.token
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl Policy for SasTokenCredentialPolicy {
    async fn send(
        &self,
        ctx: &Context,
        request: &mut Request,
        next: &[Arc<dyn Policy>],
    ) -> PolicyResult {
        // Adding the SAS token as a query parameter to the request URL.
        let query = request.url().query().unwrap_or_default();
        let query = if query.is_empty() {
            self.token.clone()
        } else {
            format!("{}&{}", self.token, query)
        };
        request.url_mut().set_query(Some(&query));

        next[0].send(ctx, request, &next[1..]).await
    }
}
