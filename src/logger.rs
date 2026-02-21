use std::fs::OpenOptions;
use std::io::Write;
use chrono::Utc;

pub fn log_request(
    cache_status: &str,
    model: &str,
    tokens: u64,
    cost: f64,
) {
    let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S");
    let log_entry = format!(
        "{} | {:13} | {:30} | {:8} tokens | ${:.5}\n",
        timestamp, cache_status, model, tokens, cost
    );

    // Use /app/requests.log in Docker, ./requests.log locally
    let log_path = std::env::var("LOG_PATH")
        .unwrap_or_else(|_| "./requests.log".to_string());

    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = file.write_all(log_entry.as_bytes());
    } else {
        eprintln!("Failed to write to log file: {}", log_path);
    }
}