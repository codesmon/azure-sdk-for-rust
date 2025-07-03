use std::collections::HashMap;

use super::logs::log_operation_result;
use azure_core::{
    http::{Response, XmlFormat},
    Error,
};
use azure_storage_queue::{
    clients::QueueClient,
    models::{
        QueueClientGetMetadataResultHeaders, QueueClientPeekMessagesOptions,
        QueueClientReceiveMessagesOptions, QueueClientSetMetadataOptions, QueueClientUpdateOptions,
        QueueMessage, SentMessage,
    },
};

pub async fn send_message(
    queue_client: &QueueClient,
    message: &str,
) -> Result<Response<Option<SentMessage>, XmlFormat>, Error> {
    let queue_message = QueueMessage {
        message_text: Some(message.to_owned()),
    };

    queue_client
        .send_message(queue_message.try_into()?, None)
        .await
}

pub async fn send_and_delete_message(
    queue_client: &QueueClient,
    message: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = send_message(queue_client, message).await;

    if let Ok(response) = result {
        let message = response.into_body().await?;

        if let Some(message) = message {
            if let (Some(message_id), Some(pop_receipt)) = (message.message_id, message.pop_receipt)
            {
                println!(
                    "Deleting message with ID: {} and pop receipt: {}",
                    message_id, pop_receipt
                );
                let delete_result = queue_client
                    .delete_message(&message_id, &pop_receipt, None)
                    .await;
                log_operation_result(&delete_result, "delete_message");
            }
        }
    }

    Ok(())
}

pub async fn send_and_update_message(
    queue_client: &QueueClient,
    message: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = send_message(queue_client, message).await;

    if let Ok(response) = result {
        let message = response.into_body().await?;

        if let Some(message) = message {
            if let (Some(message_id), Some(pop_receipt)) = (message.message_id, message.pop_receipt)
            {
                println!(
                    "Updating message with ID: {} and pop receipt: {}",
                    message_id, pop_receipt
                );
                let queue_message = QueueMessage {
                    message_text: Some("Updated message text from Rust".to_string()),
                };
                let update_option = QueueClientUpdateOptions {
                    // Serialize the message text as bytes for the update
                    queue_message: Some(queue_message.try_into()?),
                    ..Default::default()
                };
                let update_result = queue_client
                    .update_message(&message_id.clone(), &pop_receipt, 50, Some(update_option))
                    .await;
                log_operation_result(&update_result, "update_message");
            }
        }
    }

    Ok(())
}

pub async fn set_and_get_metadata(
    queue_client: &QueueClient,
) -> Result<(), Box<dyn std::error::Error>> {
    let metadata_options = Some(QueueClientSetMetadataOptions {
        metadata: Some(HashMap::from([
            ("key1".to_string(), "value1".to_string()),
            ("key2".to_string(), "value2".to_string()),
        ])),
        ..Default::default()
    });
    let result = queue_client.set_metadata(metadata_options).await;
    log_operation_result(&result, "set_metadata");

    let result = queue_client.get_metadata(None).await;
    log_operation_result(&result, "get_metadata");

    let metadata = result.unwrap().metadata().unwrap_or_default();
    for (key, value) in metadata {
        println!("Metadata - {}: {}", key, value);
    }

    Ok(())
}

pub async fn peek_and_receive_messages(
    queue_client: &QueueClient,
) -> Result<(), Box<dyn std::error::Error>> {
    _ = send_message(queue_client, "Message 1 from Rust Queue SDK").await;
    _ = send_message(queue_client, "Message 2 from Rust Queue SDK").await;

    let options = QueueClientPeekMessagesOptions {
        number_of_messages: Some(5),
        ..Default::default()
    };

    let result = queue_client.peek_messages(Some(options)).await;
    log_operation_result(&result, "peek_messages");

    if let Ok(response) = result {
        let messages = response.into_body().await?;
        if let Some(messages) = messages.items {
            for msg in messages {
                println!(
                    "Successfully peeked message ({}): {}",
                    msg.message_id.unwrap(),
                    msg.message_text.unwrap_or_default()
                );
            }
        }
    }

    let options = QueueClientReceiveMessagesOptions {
        number_of_messages: Some(5),
        ..Default::default()
    };

    let result = queue_client.receive_messages(Some(options)).await;
    log_operation_result(&result, "receive_messages");

    if let Ok(response) = result {
        let messages = response.into_body().await?;
        if let Some(messages) = messages.items {
            for msg in messages {
                println!(
                    "Successfully received message ({}): {}",
                    msg.message_id.unwrap(),
                    msg.message_text.unwrap_or_default()
                );
            }
        }
    }

    Ok(())
}

pub async fn peek_and_receive_message(
    queue_client: &QueueClient,
) -> Result<(), Box<dyn std::error::Error>> {
    _ = send_message(queue_client, "Message 1 from Rust Queue SDK").await;
    _ = send_message(queue_client, "Message 2 from Rust Queue SDK").await;

    let options = QueueClientPeekMessagesOptions {
        number_of_messages: Some(5),
        ..Default::default()
    };

    let result = queue_client.peek_message(Some(options)).await;
    log_operation_result(&result, "peek_message");

    if let Ok(response) = result {
        let message = response.into_body().await?;
        if let Some(message) = message {
            println!(
                "Successfully peeked message ({}): {}",
                message.message_id.unwrap(),
                message.message_text.unwrap_or_default()
            );
        }
    }

    loop {
        let result = queue_client.receive_message(None).await;
        log_operation_result(&result, "receive_message");

        if let Ok(response) = result {
            let message = response.into_body().await?;
            if let Some(msg) = message {
                println!(
                    "Successfully received message ({}): {}",
                    msg.message_id.unwrap(),
                    msg.message_text.unwrap_or_default()
                );
            } else {
                // No more messages available
                break;
            }
        } else {
            // Error occurred, break the loop
            break;
        }
    }

    Ok(())
}
