// Copyright (c) Microsoft Corporation. All Rights reserved
// Licensed under the MIT license.

use super::RecoverableConnection;
use crate::common::retry_azure_operation;
use azure_core::{error::ErrorKind as AzureErrorKind, error::Result, http::Url, time::Duration};
use azure_core_amqp::{AmqpError, AmqpReceiverApis, AmqpReceiverOptions, AmqpSession, AmqpSource};
use futures::{select, FutureExt};
use std::error::Error;
use std::sync::Arc;
use tracing::{debug, warn};

pub(crate) struct RecoverableReceiver {
    recoverable_connection: Arc<RecoverableConnection>,
    source_url: Url,
    message_source: AmqpSource,
    receiver_options: AmqpReceiverOptions,
    timeout: Option<Duration>,
}

impl RecoverableReceiver {
    pub(super) fn new(
        recoverable_connection: Arc<RecoverableConnection>,
        receiver_options: AmqpReceiverOptions,
        message_source: AmqpSource,
        source_url: Url,
        timeout: Option<Duration>,
    ) -> Self {
        Self {
            source_url,
            recoverable_connection,
            receiver_options,
            message_source,
            timeout,
        }
    }

    fn should_retry_receive_operation(e: &azure_core::Error) -> bool {
        match e.kind() {
            AzureErrorKind::Amqp => {
                warn!(err=?e, "Amqp operation failed: {e}");
                if let Some(e) = e.source() {
                    debug!(err=?e, "Error: {e}");

                    if let Some(amqp_error) = e.downcast_ref::<Box<AmqpError>>() {
                        RecoverableConnection::should_retry_amqp_error(amqp_error)
                    } else if let Some(amqp_error) = e.downcast_ref::<AmqpError>() {
                        RecoverableConnection::should_retry_amqp_error(amqp_error)
                    } else {
                        debug!(err=?e, "Non AMQP error: {e}");
                        false
                    }
                } else {
                    debug!("No source error found");
                    false
                }
            }
            _ => {
                debug!(err=?e, "Non AMQP error: {e}");
                false
            }
        }
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl AmqpReceiverApis for RecoverableReceiver {
    async fn attach(
        &self,
        _session: &AmqpSession,
        _source: impl Into<AmqpSource> + Send,
        _options: Option<AmqpReceiverOptions>,
    ) -> Result<()> {
        unimplemented!("AmqpReceiverClient does not support attach operation");
    }

    async fn detach(self) -> azure_core::Result<()> {
        unimplemented!("AmqpReceiverClient does not support detach operation");
    }

    async fn set_credit_mode(
        &self,
        _mode: azure_core_amqp::ReceiverCreditMode,
    ) -> azure_core::Result<()> {
        unimplemented!("AmqpReceiverClient does not support set_credit_mode operation");
    }

    async fn credit_mode(&self) -> azure_core::Result<azure_core_amqp::ReceiverCreditMode> {
        unimplemented!("AmqpReceiverClient does not support credit_mode operation");
    }

    async fn receive_delivery(&self) -> azure_core::Result<azure_core_amqp::AmqpDelivery> {
        let delivery = retry_azure_operation(
            || async move {
                let receiver = self
                    .recoverable_connection
                    .ensure_receiver(
                        &self.source_url,
                        &self.message_source,
                        &self.receiver_options,
                    )
                    .await?;
                if let Some(delivery_timeout) = self.timeout {
                    select! {
                        delivery = receiver.receive_delivery().fuse() => Ok(delivery),
                        _ = azure_core::sleep::sleep(delivery_timeout).fuse() => {
                             Err(azure_core::Error::new(
                                AzureErrorKind::Io,
                                Box::new(std::io::Error::from(std::io::ErrorKind::TimedOut))))
                        },
                    }?
                } else {
                    receiver.receive_delivery().await
                }
            },
            &self.recoverable_connection.retry_options,
            Some(Self::should_retry_receive_operation),
        )
        .await?;
        Ok(delivery)
    }

    async fn accept_delivery(
        &self,
        _delivery: &azure_core_amqp::AmqpDelivery,
    ) -> azure_core::Result<()> {
        unimplemented!("AmqpReceiverClient does not support accept_delivery operation");
    }

    async fn reject_delivery(
        &self,
        _delivery: &azure_core_amqp::AmqpDelivery,
    ) -> azure_core::Result<()> {
        unimplemented!("AmqpReceiverClient does not support reject_delivery operation");
    }

    async fn release_delivery(
        &self,
        _delivery: &azure_core_amqp::AmqpDelivery,
    ) -> azure_core::Result<()> {
        unimplemented!("AmqpReceiverClient does not support release_delivery operation");
    }
}
