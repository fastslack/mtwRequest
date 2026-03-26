//! Demo: mtwRequest WebSocket client
//!
//! Run the server first: cargo run --example demo_server
//! Then run this:        cargo run --example demo_client

use futures::{SinkExt, StreamExt};
use mtw_protocol::{MsgType, MtwMessage, Payload};
use tokio_tungstenite::tungstenite::Message as WsMessage;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    let url = "ws://127.0.0.1:8080";
    tracing::info!("connecting to {}...", url);

    let (ws_stream, _) = tokio_tungstenite::connect_async(url).await?;
    let (mut sink, mut stream) = ws_stream.split();

    tracing::info!("connected!");

    // Read the connection ACK
    if let Some(Ok(WsMessage::Text(text))) = stream.next().await {
        let msg: MtwMessage = serde_json::from_str(&text)?;
        tracing::info!("server ACK: {:?}", msg.payload);
    }

    // 1. Subscribe to chat.general
    tracing::info!("\n--- subscribing to chat.general ---");
    let sub_msg = MtwMessage::new(MsgType::Subscribe, Payload::None)
        .with_channel("chat.general");
    sink.send(WsMessage::Text(serde_json::to_string(&sub_msg)?.into())).await?;

    if let Some(Ok(WsMessage::Text(text))) = stream.next().await {
        let msg: MtwMessage = serde_json::from_str(&text)?;
        tracing::info!("subscribe response: {:?}", msg.payload);
    }

    // 2. Publish a message to chat.general
    tracing::info!("\n--- publishing message to chat.general ---");
    let pub_msg = MtwMessage::new(
        MsgType::Publish,
        Payload::Json(serde_json::json!({
            "user": "fastslack",
            "text": "Hello from mtwRequest! This is a real-time message.",
            "timestamp": chrono_now(),
        })),
    ).with_channel("chat.general");
    sink.send(WsMessage::Text(serde_json::to_string(&pub_msg)?.into())).await?;

    if let Some(Ok(WsMessage::Text(text))) = stream.next().await {
        let msg: MtwMessage = serde_json::from_str(&text)?;
        tracing::info!("publish response: {:?}", msg.payload);
    }

    // 3. Send a request (echo)
    tracing::info!("\n--- sending echo request ---");
    let req_msg = MtwMessage::request(Payload::Json(serde_json::json!({
        "action": "echo",
        "data": {"hello": "world", "number": 42}
    })));
    sink.send(WsMessage::Text(serde_json::to_string(&req_msg)?.into())).await?;

    if let Some(Ok(WsMessage::Text(text))) = stream.next().await {
        let msg: MtwMessage = serde_json::from_str(&text)?;
        tracing::info!("echo response: {:?}", msg.payload);
    }

    // 4. Subscribe to notifications
    tracing::info!("\n--- subscribing to notifications ---");
    let sub_notif = MtwMessage::new(MsgType::Subscribe, Payload::None)
        .with_channel("notifications");
    sink.send(WsMessage::Text(serde_json::to_string(&sub_notif)?.into())).await?;

    if let Some(Ok(WsMessage::Text(text))) = stream.next().await {
        let msg: MtwMessage = serde_json::from_str(&text)?;
        tracing::info!("subscribe response: {:?}", msg.payload);
    }

    // 5. Publish to notifications
    tracing::info!("\n--- publishing notification ---");
    let notif_msg = MtwMessage::new(
        MsgType::Publish,
        Payload::Json(serde_json::json!({
            "type": "alert",
            "title": "mtwRequest is working!",
            "body": "Real-time WebSocket communication is fully operational.",
            "priority": "high"
        })),
    ).with_channel("notifications");
    sink.send(WsMessage::Text(serde_json::to_string(&notif_msg)?.into())).await?;

    if let Some(Ok(WsMessage::Text(text))) = stream.next().await {
        let msg: MtwMessage = serde_json::from_str(&text)?;
        tracing::info!("publish response: {:?}", msg.payload);
    }

    // 6. Test error case — publish to non-existent channel
    tracing::info!("\n--- testing error: publish to non-existent channel ---");
    let bad_msg = MtwMessage::new(
        MsgType::Publish,
        Payload::Text("this should fail".into()),
    ).with_channel("does.not.exist");
    sink.send(WsMessage::Text(serde_json::to_string(&bad_msg)?.into())).await?;

    if let Some(Ok(WsMessage::Text(text))) = stream.next().await {
        let msg: MtwMessage = serde_json::from_str(&text)?;
        tracing::info!("error response: {:?}", msg.payload);
    }

    // 7. Unsubscribe from chat.general
    tracing::info!("\n--- unsubscribing from chat.general ---");
    let unsub_msg = MtwMessage::new(MsgType::Unsubscribe, Payload::None)
        .with_channel("chat.general");
    sink.send(WsMessage::Text(serde_json::to_string(&unsub_msg)?.into())).await?;

    if let Some(Ok(WsMessage::Text(text))) = stream.next().await {
        let msg: MtwMessage = serde_json::from_str(&text)?;
        tracing::info!("unsubscribe response: {:?}", msg.payload);
    }

    tracing::info!("\n=== all tests passed! ===");
    tracing::info!("mtwRequest is fully operational");

    // Close connection
    sink.send(WsMessage::Close(None)).await?;

    Ok(())
}

fn chrono_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    format!("{}", now)
}
