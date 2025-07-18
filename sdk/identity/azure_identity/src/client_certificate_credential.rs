// Copyright (c) Microsoft Corporation. All rights reserved.
// Licensed under the MIT License.

use crate::{EntraIdTokenResponse, TokenCache, TokenCredentialOptions};
use azure_core::{
    base64,
    credentials::{AccessToken, Secret, TokenCredential, TokenRequestOptions},
    error::{http_response_from_body, Error, ErrorKind},
    http::{
        headers::{self, content_type},
        request::Request,
        HttpClient, Method, Url,
    },
    time::{Duration, OffsetDateTime},
    Uuid,
};

// cspell:ignore pkey
use openssl::{
    error::ErrorStack,
    hash::{hash, DigestBytes, MessageDigest},
    pkcs12::Pkcs12,
    pkey::{PKey, Private},
    sign::Signer,
    x509::X509,
};
use std::{str, sync::Arc};

use url::form_urlencoded;

/// Refresh time to use in seconds.
const DEFAULT_REFRESH_TIME: i64 = 300;

const AZURE_CLIENT_SEND_CERTIFICATE_CHAIN_ENV_KEY: &str = "AZURE_CLIENT_SEND_CERTIFICATE_CHAIN";

/// Provides options to configure how the Identity library makes authentication
/// requests to Azure Active Directory.
#[derive(Clone, Debug)]
pub struct ClientCertificateCredentialOptions {
    options: TokenCredentialOptions,
    send_certificate_chain: bool,
}

impl From<TokenCredentialOptions> for ClientCertificateCredentialOptions {
    fn from(options: TokenCredentialOptions) -> Self {
        let env = options.env();
        let send_certificate_chain = env
            .var(AZURE_CLIENT_SEND_CERTIFICATE_CHAIN_ENV_KEY)
            .map(|s| s == "1" || s.to_lowercase() == "true")
            .unwrap_or(false);
        Self {
            options,
            send_certificate_chain,
        }
    }
}

impl ClientCertificateCredentialOptions {
    /// Create a new `TokenCredentialsOptions`. default() may also be used.
    pub fn new(options: impl Into<TokenCredentialOptions>, send_certificate_chain: bool) -> Self {
        Self {
            options: options.into(),
            send_certificate_chain,
        }
    }

    pub fn options(&self) -> &TokenCredentialOptions {
        &self.options
    }

    pub fn options_mut(&mut self) -> &mut TokenCredentialOptions {
        &mut self.options
    }

    /// Enable/disable sending the certificate chain.
    pub fn set_send_certificate_chain(&mut self, send_certificate_chain: bool) {
        self.send_certificate_chain = send_certificate_chain;
    }

    /// Whether certificate chain is sent as part of the request or not. Default is
    /// set to true.
    pub fn send_certificate_chain(&self) -> bool {
        self.send_certificate_chain
    }
}

/// Enables authentication to Azure Active Directory using a client certificate that
/// was generated for an App Registration.
///
/// In order to use subject name validation `send_cert_chain` option must be set to true
/// The certificate is expected to be in base64 encoded PKCS12 format.
#[derive(Debug)]
pub struct ClientCertificateCredential {
    tenant_id: String,
    client_id: String,
    client_certificate: Secret,
    client_certificate_pass: Secret,
    http_client: Arc<dyn HttpClient>,
    authority_host: Url,
    send_certificate_chain: bool,
    cache: TokenCache,
}

impl ClientCertificateCredential {
    /// Create a new `ClientCertificateCredential`.
    pub fn new<C, P>(
        tenant_id: String,
        client_id: String,
        client_certificate: C,
        client_certificate_pass: P,
        options: impl Into<ClientCertificateCredentialOptions>,
    ) -> azure_core::Result<Arc<ClientCertificateCredential>>
    where
        C: Into<Secret>,
        P: Into<Secret>,
    {
        let options = options.into();
        Ok(Arc::new(ClientCertificateCredential {
            tenant_id,
            client_id,
            client_certificate: client_certificate.into(),
            client_certificate_pass: client_certificate_pass.into(),
            http_client: options.options().http_client().clone(),
            authority_host: options.options().authority_host()?.clone(),
            send_certificate_chain: options.send_certificate_chain(),
            cache: TokenCache::new(),
        }))
    }

    fn sign(jwt: &str, pkey: &PKey<Private>) -> Result<Vec<u8>, ErrorStack> {
        let mut signer = Signer::new(MessageDigest::sha256(), pkey)?;
        signer.update(jwt.as_bytes())?;
        signer.sign_to_vec()
    }

    fn get_thumbprint(cert: &X509) -> Result<DigestBytes, ErrorStack> {
        let der = cert.to_der()?;
        let digest = hash(MessageDigest::sha1(), &der)?;
        Ok(digest)
    }

    fn as_jwt_part(part: &[u8]) -> String {
        base64::encode_url_safe(part)
    }

    async fn get_token(
        &self,
        scopes: &[&str],
        _: Option<TokenRequestOptions>,
    ) -> azure_core::Result<AccessToken> {
        if scopes.len() != 1 {
            return Err(Error::message(
                ErrorKind::Credential,
                "only one scope is supported for IMDS authentication",
            ));
        }

        let Some(scope) = scopes.first() else {
            return Err(Error::message(
                ErrorKind::Credential,
                "no scopes were provided",
            ));
        };

        let url = self
            .authority_host
            .join(&format!("{}/oauth2/v2.0/token", self.tenant_id))?;

        let certificate = base64::decode(self.client_certificate.secret())
            .map_err(|_| Error::message(ErrorKind::Credential, "Base64 decode failed"))?;

        let pkcs12_certificate = Pkcs12::from_der(&certificate)
            .map_err(openssl_error)?
            .parse2(self.client_certificate_pass.secret())
            .map_err(openssl_error)?;

        let Some(cert) = pkcs12_certificate.cert.as_ref() else {
            return Err(Error::message(
                ErrorKind::Credential,
                "Certificate not found",
            ));
        };

        let Some(pkey) = pkcs12_certificate.pkey.as_ref() else {
            return Err(Error::message(
                ErrorKind::Credential,
                "Private key not found",
            ));
        };

        let thumbprint =
            ClientCertificateCredential::get_thumbprint(cert).map_err(openssl_error)?;

        let uuid = Uuid::new_v4();
        let current_time = OffsetDateTime::now_utc().unix_timestamp();
        let expiry_time = current_time + DEFAULT_REFRESH_TIME;
        let x5t = base64::encode(thumbprint);

        let header = match self.send_certificate_chain {
            true => {
                let base_signature = get_encoded_cert(cert)?;
                let x5c = match pkcs12_certificate.ca {
                    Some(chain) => {
                        let chain = chain
                            .into_iter()
                            .map(|x| get_encoded_cert(&x))
                            .collect::<azure_core::Result<Vec<String>>>()?
                            .join(",");
                        format! {"{},{}", base_signature, chain}
                    }
                    None => base_signature,
                };
                format!(
                    r#"{{"alg":"RS256","typ":"JWT", "x5t":"{}", "x5c":[{}]}}"#,
                    x5t, x5c
                )
            }
            false => format!(r#"{{"alg":"RS256","typ":"JWT", "x5t":"{}"}}"#, x5t),
        };
        let header = ClientCertificateCredential::as_jwt_part(header.as_bytes());

        let payload = format!(
            r#"{{"aud":"{}","exp":{},"iss": "{}", "jti": "{}", "nbf": {}, "sub": "{}"}}"#,
            url, expiry_time, self.client_id, uuid, current_time, self.client_id
        );
        let payload = ClientCertificateCredential::as_jwt_part(payload.as_bytes());

        let jwt = format!("{}.{}", header, payload);
        let signature = ClientCertificateCredential::sign(&jwt, pkey).map_err(openssl_error)?;
        let sig = ClientCertificateCredential::as_jwt_part(&signature);
        let client_assertion = format!("{}.{}", jwt, sig);

        let encoded = {
            let mut encoded = &mut form_urlencoded::Serializer::new(String::new());
            encoded = encoded
                .append_pair("client_id", self.client_id.as_str())
                .append_pair("scope", scope)
                .append_pair(
                    "client_assertion_type",
                    "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
                )
                .append_pair("client_assertion", client_assertion.as_str())
                .append_pair("grant_type", "client_credentials");
            encoded.finish()
        };

        let mut req = Request::new(url, Method::Post);
        req.insert_header(
            headers::CONTENT_TYPE,
            content_type::APPLICATION_X_WWW_FORM_URLENCODED,
        );
        req.set_body(encoded);

        let rsp = self.http_client.execute_request(&req).await?;
        let rsp_status = rsp.status();

        if !rsp_status.is_success() {
            let rsp_body = rsp.into_body().collect().await?;
            return Err(http_response_from_body(rsp_status, &rsp_body).into_error());
        }

        let response: EntraIdTokenResponse = rsp.into_body().json().await?;
        Ok(AccessToken::new(
            response.access_token,
            OffsetDateTime::now_utc() + Duration::seconds(response.expires_in),
        ))
    }
}

fn get_encoded_cert(cert: &X509) -> azure_core::Result<String> {
    Ok(format!(
        "\"{}\"",
        base64::encode(cert.to_pem().map_err(openssl_error)?)
    ))
}

fn openssl_error(err: ErrorStack) -> azure_core::error::Error {
    Error::new(ErrorKind::Credential, err)
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl TokenCredential for ClientCertificateCredential {
    async fn get_token(
        &self,
        scopes: &[&str],
        options: Option<TokenRequestOptions>,
    ) -> azure_core::Result<AccessToken> {
        self.cache
            .get_token(scopes, options, |s, o| self.get_token(s, o))
            .await
    }
}
