use picstome_sync_core::mock::{start_mock_server, MockServerOptions};

#[tokio::main]
async fn main() {
    let options = MockServerOptions {
        host: std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".into()),
        port: std::env::var("PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(8787),
        photo_size: std::env::var("PHOTO_SIZE")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(32 * 1024),
        token: std::env::var("MOCK_TOKEN").unwrap_or_else(|_| "demo-token".into()),
        latency_ms: std::env::var("LATENCY_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0),
    };

    let mock = match start_mock_server(options).await {
        Ok(mock) => mock,
        Err(err) => {
            eprintln!("Could not start mock server: {err}");
            std::process::exit(1);
        }
    };

    println!();
    println!("  Picstome fake API is running");
    println!("  Web demo:  {}", mock.url);
    println!("  API base:  {}", mock.url);
    println!("  Token:     {}", mock.token);
    println!();
    println!("  Start the app with:");
    println!(
        "    PICSTOME_API_BASE={} PICSTOME_MOCK=1 cargo run -p picstome-sync",
        mock.url
    );
    println!("  Or simply run: cargo run -p picstome-sync -- --demo");
    println!();
    println!("  Press Ctrl+C to stop.");
    println!();

    let _ = tokio::signal::ctrl_c().await;
    mock.close().await;
}
