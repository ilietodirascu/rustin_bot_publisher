use axum::{debug_handler, http::StatusCode, Extension, Json};
use lapin::{options::BasicPublishOptions, BasicProperties, Channel};
use log::info;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{iter::Cycle, sync::Arc, vec::IntoIter};
use tokio::sync::Mutex;

pub struct ChannelPool {
    channels: Mutex<Cycle<IntoIter<Arc<Channel>>>>,
}

impl ChannelPool {
    pub fn new(channels: Vec<Arc<Channel>>) -> Self {
        let channel_iter = channels.into_iter().cycle();
        Self {
            channels: Mutex::new(channel_iter),
        }
    }

    pub async fn get_next_channel(&self) -> Arc<Channel> {
        let mut channels = self.channels.lock().await;
        channels.next().expect("Channel pool should never be empty")
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct RabbitMessage {
    chat_id: i64,
    text: String,
}

#[debug_handler]
pub async fn receive_message(
    Extension(channel_pool): Extension<Arc<ChannelPool>>,
    Json(payload): Json<Value>,
) -> Result<StatusCode, StatusCode> {
    info!("Received message payload: {:?}", payload);

    if let Some(chat_id) = extract_chat_id(&payload) {
        if let Some(command) = extract_caption(&payload) {
            match command {
                "/readimage" => handle_readimage(chat_id, &payload, &channel_pool).await?,
                _ => return Ok(StatusCode::OK),
            }
        } else if let Some(text) = extract_text(&payload) {
            if text == "/help" {
                handle_help_command(chat_id, &channel_pool).await?;
            } else if text.starts_with("/songlinks") {
                handle_songlinks(chat_id, text, &channel_pool).await?;
            }
        }
    } else {
        info!("No valid chat_id found in the message payload.");
        return Err(StatusCode::BAD_REQUEST);
    }

    Ok(StatusCode::OK)
}

// Extract chat_id from the payload
fn extract_chat_id(payload: &Value) -> Option<i64> {
    payload["message"]["chat"]["id"].as_i64()
}

// Extract caption from the payload (used for commands like /readimage)
fn extract_caption(payload: &Value) -> Option<&str> {
    payload["message"]["caption"].as_str()
}

// Extract text from the payload (used for /help and other text commands)
fn extract_text(payload: &Value) -> Option<&str> {
    payload["message"]["text"].as_str()
}

// Handle the /readimage command by sending the file_id to the ImageToText queue
async fn handle_readimage(
    chat_id: i64,
    payload: &Value,
    channel_pool: &Arc<ChannelPool>,
) -> Result<(), StatusCode> {
    if let Some(file_id) = extract_largest_image_file_id(payload) {
        let rabbit_message = RabbitMessage {
            chat_id,
            text: file_id.to_string(),
        };
        publish_to_queue("ImageToText", rabbit_message, channel_pool).await?;
        info!("Published 'readimage' message to ImageToText queue.");
        Ok(())
    } else {
        info!("No valid file_id found in the photo.");
        Err(StatusCode::BAD_REQUEST)
    }
}

// Handle the /help command by sending a help message to the Reply queue
async fn handle_help_command(
    chat_id: i64,
    channel_pool: &Arc<ChannelPool>,
) -> Result<(), StatusCode> {
    let help_message = RabbitMessage {
        chat_id,
        text: "Type /songlinks, followed by up to 10 lines of song titles to get download links.\n/readimage with an attached image, to get the text from the image.\n/donate to get a QR code."
            .to_string(),
    };
    publish_to_queue("Reply", help_message, channel_pool).await?;
    info!("Published 'help' message to Reply queue.");
    Ok(())
}

// Extract the file_id of the largest image from the payload
fn extract_largest_image_file_id(payload: &Value) -> Option<&str> {
    payload["message"]["photo"]
        .as_array()?
        .iter()
        .max_by_key(|p| p["width"].as_i64().unwrap_or(0))
        .and_then(|photo| photo["file_id"].as_str())
}

// Publish a RabbitMessage to the specified RabbitMQ queue
async fn publish_to_queue(
    queue_name: &str,
    message: RabbitMessage,
    channel_pool: &Arc<ChannelPool>,
) -> Result<(), StatusCode> {
    let serialized_message = serde_json::to_vec(&message).expect("Failed to serialize message");
    let channel = channel_pool.get_next_channel().await;
    channel
        .basic_publish(
            "",         // Exchange
            queue_name, // Queue name
            BasicPublishOptions::default(),
            &serialized_message, // Payload
            BasicProperties::default(),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(())
}
async fn handle_songlinks(
    chat_id: i64,
    text: &str,
    channel_pool: &Arc<ChannelPool>,
) -> Result<(), StatusCode> {
    // Extract song lines, skipping the /songlinks command
    let truncated_songs: Vec<String> = text
        .lines()
        .skip(1) // Skip the /songlinks command itself
        .take(10) // Limit to 10 lines
        .map(|line| line.chars().take(50).collect()) // Truncate each line to 50 characters
        .collect();

    let song_message = RabbitMessage {
        chat_id,
        text: truncated_songs.join("\n"), // Join all truncated lines with newlines
    };

    publish_to_queue("Music", song_message, channel_pool).await?;
    info!("Published 'songlinks' message to Music queue.");
    Ok(())
}
