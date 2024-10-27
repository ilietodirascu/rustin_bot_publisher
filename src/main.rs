use std::{env, sync::Arc};

use axum::{
    response::IntoResponse,
    routing::{get, post},
    Extension, Router,
};
use dotenvy::dotenv;
use lapin::{Connection, ConnectionProperties};
use webhook_handler::{receive_message, ChannelPool};
pub mod webhook_handler;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    dotenv().expect("Failed to load .env file");
    let server_address = env::var("SERVER_ADDRESS").expect("SERVER_ADDRESS must be set");

    let rabbit_addr = env::var("RABBIT_ADDRESS").expect("RABBIT_ADDRESS must be set");

    let connection = Connection::connect(&rabbit_addr, ConnectionProperties::default())
        .await
        .expect("Failed to connect to RabbitMQ");

    // Create a pool of RabbitMQ channels (e.g., 5 channels)
    let mut channels = Vec::new();
    for _ in 0..5 {
        let channel = Arc::new(
            connection
                .create_channel()
                .await
                .expect("Failed to create channel"),
        );
        channels.push(channel);
    }

    // Create the channel pool using the cycling iterator
    let channel_pool = Arc::new(ChannelPool::new(channels));

    let app = Router::new()
        .route("/", get(hello))
        .route("/webhook", post(receive_message))
        .layer(Extension(Arc::clone(&channel_pool)));
    let listener = tokio::net::TcpListener::bind(server_address)
        .await
        .expect("Could not bind to address");

    println!("Listening on {}", listener.local_addr().unwrap());

    axum::serve(listener, app)
        .await
        .expect("Error serving application");
}
async fn hello() -> impl IntoResponse {
    "Hello"
}
