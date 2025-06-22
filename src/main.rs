//! # MailHits - Main Application
//!
//! This is the main entry point for the MailHits application.
//! It starts both the SMTP server for capturing emails and the HTTP server for the web interface.

pub mod http;
pub mod models;
pub mod smtp;

use clap::Parser;
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;
use tracing_subscriber;

use crate::models::AppState;

/// Command line arguments for the application
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// SMTP server port
    #[arg(short, long, default_value_t = 1025)]
    smtp_port: u16,

    /// HTTP server port
    #[arg(short = 'p', long, default_value_t = 3000)]
    http_port: u16,
}

/// Creates the application state
fn create_app_state() -> Arc<AppState> {
    let (tx, _) = broadcast::channel(100);
    Arc::new(AppState {
        emails: RwLock::new(Vec::new()),
        tx,
    })
}

/// Main entry point for the application
///
/// Initializes logging, parses command line arguments, creates the application state,
/// and starts both the SMTP and HTTP servers.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

    // Parse command-line arguments
    let args = Args::parse();

    // Create application state
    let state = create_app_state();

    // Start SMTP server in a separate task
    let smtp_state = state.clone();
    let smtp_port = args.smtp_port;
    tokio::spawn(async move {
        smtp::start_smtp_server(smtp_state, smtp_port).await;
    });

    // Start HTTP server (this will block until the server shuts down)
    http::start_http_server(state, args.http_port).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_args_default_values() {
        let args = Args::parse_from(["mailhits"]);
        assert_eq!(args.smtp_port, 1025);
        assert_eq!(args.http_port, 3000);
    }

    #[test]
    fn test_args_custom_values() {
        let args = Args::parse_from(["mailhits", "-s", "2025", "-p", "4000"]);
        assert_eq!(args.smtp_port, 2025);
        assert_eq!(args.http_port, 4000);
    }

    #[test]
    fn test_create_app_state() {
        let state = create_app_state();
        let emails = state.emails.read().unwrap();
        assert_eq!(emails.len(), 0);
    }
}
