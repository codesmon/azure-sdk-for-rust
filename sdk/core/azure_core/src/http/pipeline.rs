// Copyright (c) Microsoft Corporation. All rights reserved.
// Licensed under the MIT License.

use super::policies::ClientRequestIdPolicy;
use crate::http::{
    policies::{Policy, UserAgentPolicy},
    ClientOptions,
};
use std::{
    any::{Any, TypeId},
    ops::Deref,
    sync::Arc,
};
use typespec_client_core::http;

/// Execution pipeline.
///
/// A pipeline follows a precise flow:
///
/// 1. Client library-specified per-call policies are executed. Per-call policies can fail and bail out of the pipeline
///    immediately.
/// 2. User-specified per-call policies in [`ClientOptions::per_call_policies`] are executed.
/// 3. The retry policy is executed. It allows to re-execute the following policies.
/// 4. The [`CustomHeadersPolicy`](crate::http::policies::CustomHeadersPolicy) is executed
/// 5. Client library-specified per-retry policies. Per-retry polices are always executed at least once but are
///    re-executed in case of retries.
/// 6. User-specified per-retry policies in [`ClientOptions::per_try_policies`] are executed.
/// 7. The transport policy is executed. Transport policy is always the last policy and is the policy that
///    actually constructs the [`RawResponse`](http::RawResponse) to be passed up the pipeline.
///
/// A pipeline is immutable. In other words a policy can either succeed and call the following
/// policy of fail and return to the calling policy. Arbitrary policy "skip" must be avoided (but
/// cannot be enforced by code). All policies except Transport policy can assume there is another following policy (so
/// `self.pipeline[0]` is always valid).
#[derive(Debug, Clone)]
pub struct Pipeline(http::Pipeline);

impl Pipeline {
    /// Creates a new pipeline given the client library crate name and version,
    /// alone with user-specified and client library-specified policies.
    ///
    /// Crates can simply pass `option_env!("CARGO_PKG_NAME")` and `option_env!("CARGO_PKG_VERSION")` for the
    /// `crate_name` and `crate_version` arguments respectively.
    pub fn new(
        crate_name: Option<&'static str>,
        crate_version: Option<&'static str>,
        options: ClientOptions,
        per_call_policies: Vec<Arc<dyn Policy>>,
        per_try_policies: Vec<Arc<dyn Policy>>,
    ) -> Self {
        let mut per_call_policies = per_call_policies.clone();
        push_unique(&mut per_call_policies, ClientRequestIdPolicy::default());

        let (user_agent, options) = options.deconstruct();
        if !user_agent.disabled {
            let telemetry_policy = UserAgentPolicy::new(crate_name, crate_version, &user_agent);
            push_unique(&mut per_call_policies, telemetry_policy);
        }

        Self(http::Pipeline::new(
            options,
            per_call_policies,
            per_try_policies,
        ))
    }
}

#[inline]
fn push_unique<T: Policy + 'static>(policies: &mut Vec<Arc<dyn Policy>>, policy: T) {
    if policies.iter().all(|p| TypeId::of::<T>() != p.type_id()) {
        policies.push(Arc::new(policy));
    }
}

// TODO: Should we instead use the newtype pattern?
impl Deref for Pipeline {
    type Target = http::Pipeline;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        http::{
            headers::{self, HeaderName, Headers},
            policies::Policy,
            request::options::ClientRequestId,
            ClientOptions, Context, Method, RawResponse, Request, StatusCode, TransportOptions,
            UserAgentOptions,
        },
        Bytes,
    };
    use azure_core_test::http::MockHttpClient;
    use futures::FutureExt as _;
    use std::sync::Arc;

    #[tokio::test]
    async fn pipeline_with_custom_client_request_id_policy() {
        // Arrange
        const CUSTOM_HEADER_NAME: &str = "x-custom-request-id";
        const CUSTOM_HEADER: HeaderName = HeaderName::from_static(CUSTOM_HEADER_NAME);
        const CLIENT_REQUEST_ID: &str = "custom-request-id";

        let mut ctx = Context::new();
        ctx.insert(ClientRequestId::new(CLIENT_REQUEST_ID.to_string()));

        let transport = TransportOptions::new(Arc::new(MockHttpClient::new(|req| {
            async {
                // Assert
                let header_value = req
                    .headers()
                    .get_optional_str(&CUSTOM_HEADER)
                    .expect("Custom header should be present");
                assert_eq!(
                    header_value, CLIENT_REQUEST_ID,
                    "Custom header value should match the client request ID"
                );

                Ok(RawResponse::from_bytes(
                    StatusCode::Ok,
                    Headers::new(),
                    Bytes::new(),
                ))
            }
            .boxed()
        })));
        let options = ClientOptions {
            transport: Some(transport),
            ..Default::default()
        };

        let per_call_policies: Vec<Arc<dyn Policy>> =
            vec![
                Arc::new(ClientRequestIdPolicy::with_header_name(CUSTOM_HEADER_NAME))
                    as Arc<dyn Policy>,
            ];
        let per_retry_policies = vec![];

        let pipeline = Pipeline::new(
            Some("test-crate"),
            Some("1.0.0"),
            options,
            per_call_policies,
            per_retry_policies,
        );

        let mut request = Request::new("https://example.com".parse().unwrap(), Method::Get);

        // Act
        pipeline
            .send(&ctx, &mut request)
            .await
            .expect("Pipeline execution failed");
    }

    #[tokio::test]
    async fn pipeline_without_client_request_id_policy() {
        // Arrange
        const CLIENT_REQUEST_ID: &str = "default-request-id";

        let mut ctx = Context::new();
        ctx.insert(ClientRequestId::new(CLIENT_REQUEST_ID.to_string()));

        let transport = TransportOptions::new(Arc::new(MockHttpClient::new(|req| {
            async {
                // Assert
                let header_value = req
                    .headers()
                    .get_optional_str(&headers::CLIENT_REQUEST_ID)
                    .expect("Default header should be present");
                assert_eq!(
                    header_value, CLIENT_REQUEST_ID,
                    "Default header value should match the client request ID"
                );

                Ok(RawResponse::from_bytes(
                    StatusCode::Ok,
                    Headers::new(),
                    Bytes::new(),
                ))
            }
            .boxed()
        })));
        let options = ClientOptions {
            transport: Some(transport),
            ..Default::default()
        };

        let per_call_policies = vec![]; // No ClientRequestIdPolicy added
        let per_retry_policies = vec![];

        let pipeline = Pipeline::new(
            Some("test-crate"),
            Some("1.0.0"),
            options,
            per_call_policies,
            per_retry_policies,
        );

        let mut request = Request::new("https://example.com".parse().unwrap(), Method::Get);

        // Act
        pipeline
            .send(&ctx, &mut request)
            .await
            .expect("Pipeline execution failed");
    }

    #[tokio::test]
    async fn pipeline_with_user_agent_enabled_default() {
        // Arrange
        let ctx = Context::new();

        let transport = TransportOptions::new(Arc::new(MockHttpClient::new(|req| {
            async {
                // Assert
                let user_agent = req
                    .headers()
                    .get_optional_str(&headers::USER_AGENT)
                    .expect("User-Agent header should be present by default");
                // The default user agent format is: azsdk-rust-<crate_name>/<crate_version> (<rustc_version>; <OS>; <ARCH>)
                // Since we can't know the rustc version at runtime, just check the prefix and crate/version
                assert!(
                    user_agent.starts_with("azsdk-rust-test-crate/1.0.0 "),
                    "User-Agent header should start with expected prefix, got: {}",
                    user_agent
                );

                Ok(RawResponse::from_bytes(
                    StatusCode::Ok,
                    Headers::new(),
                    Bytes::new(),
                ))
            }
            .boxed()
        })));
        let options = ClientOptions {
            transport: Some(transport),
            ..Default::default()
        };

        let per_call_policies = vec![];
        let per_retry_policies = vec![];

        let pipeline = Pipeline::new(
            Some("test-crate"),
            Some("1.0.0"),
            options,
            per_call_policies,
            per_retry_policies,
        );

        let mut request = Request::new("https://example.com".parse().unwrap(), Method::Get);

        // Act
        pipeline
            .send(&ctx, &mut request)
            .await
            .expect("Pipeline execution failed");
    }

    #[tokio::test]
    async fn pipeline_with_user_agent_disabled() {
        // Arrange
        let ctx = Context::new();

        let transport = TransportOptions::new(Arc::new(MockHttpClient::new(|req| {
            async {
                // Assert
                let user_agent = req.headers().get_optional_str(&headers::USER_AGENT);
                assert!(
                    user_agent.is_none(),
                    "User-Agent header should not be present when disabled"
                );

                Ok(RawResponse::from_bytes(
                    StatusCode::Ok,
                    Headers::new(),
                    Bytes::new(),
                ))
            }
            .boxed()
        })));
        let user_agent = UserAgentOptions {
            disabled: true,
            ..Default::default()
        };
        let options = ClientOptions {
            transport: Some(transport),
            user_agent: Some(user_agent),
            ..Default::default()
        };

        let per_call_policies = vec![];
        let per_retry_policies = vec![];

        let pipeline = Pipeline::new(
            Some("test-crate"),
            Some("1.0.0"),
            options,
            per_call_policies,
            per_retry_policies,
        );

        let mut request = Request::new("https://example.com".parse().unwrap(), Method::Get);

        // Act
        pipeline
            .send(&ctx, &mut request)
            .await
            .expect("Pipeline execution failed");
    }
}
