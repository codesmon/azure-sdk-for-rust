mod helpers;
use helpers::{
    endpoint::get_endpoint, logs::log_operation_result, queue_client_operations::*,
    random_queue_name::get_random_queue_name,
};

use azure_identity::DefaultAzureCredential;
use azure_storage_queue::clients::QueueClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let credential = DefaultAzureCredential::new()?;

    // Retrieve the storage account endpoint from environment variable.
    let endpoint = get_endpoint();

    let queue_name = get_random_queue_name();
    let queue_client = QueueClient::new(&endpoint, &queue_name, credential.clone(), None)?;
    // let queue_client = QueueClient::from_sas_token(
    //     &endpoint,
    //     &queue_name,
    //     "sv=2024-11-04&ss=bfqt&srt=sco&sp=rwdlacupiytfx&se=2025-07-03T19:22:17Z&st=2025-07-03T11:22:17Z&spr=https&sig=REDACTED_SIGNATURE",
    //     None
    // )?;

    // Create and manage queue
    let result = queue_client.create(None).await;
    log_operation_result(&result, "create");

    let result = queue_client.exists().await;
    log_operation_result(&result, "check_exists");

    let result = queue_client.create_if_not_exists(None).await;
    log_operation_result(&result, "create_if_not_exists");

    // Set and get queue metadata
    set_and_get_metadata(&queue_client).await?;

    let result = send_message(&queue_client, "Example Message").await;
    log_operation_result(&result, "send_message");

    send_and_update_message(
        &queue_client,
        "Example message created from Rust, ready for update",
    )
    .await?;

    // Clear messages
    let result = queue_client.clear(None).await;
    log_operation_result(&result, "clear");

    // Send and process messages
    send_and_delete_message(
        &queue_client,
        "Example message created from Rust, ready for deletion",
    )
    .await?;

    // Peek and Receive messages
    peek_and_receive_messages(&queue_client).await?;

    // Peek and Receive message
    peek_and_receive_message(&queue_client).await?;

    // Cleanup
    let result = queue_client.delete(None).await;
    log_operation_result(&result, "delete");

    let non_existing_queue_client =
        QueueClient::new(&endpoint, "non-existent-queue", credential.clone(), None)?;
    let result = non_existing_queue_client.exists().await;
    log_operation_result(&result, "check_non_existent");

    let result = non_existing_queue_client.delete_if_exists(None).await;
    log_operation_result(&result, "delete_if_exists");

    Ok(())
}
