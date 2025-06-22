use std::sync::Arc;

use axum::extract::ws::Utf8Bytes;
use axum::{
    Json, Router,
    extract::{Path, State, WebSocketUpgrade},
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use mime_guess::from_path;
use rust_embed::RustEmbed;
use tower_http::cors::CorsLayer;

use crate::models::{AppState, Email};

/// Static assets embedded in the binary
#[derive(RustEmbed)]
#[folder = "static/"]
struct StaticAssets;

/// API Handlers for the HTTP server

/// Get all captured emails
///
/// Returns a JSON array of all emails in the system
pub async fn get_emails(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let emails = state.emails.read().unwrap();

    Json(emails.clone())
}

/// Get a specific email by ID
///
/// Returns a single email as JSON or a 404 if not found
pub async fn get_email(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Email>, StatusCode> {
    let emails = state.emails.read().unwrap();
    let email = emails
        .iter()
        .find(|e| e.id == id)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(email))
}

/// Get an attachment from an email
///
/// Returns the attachment data with appropriate content type headers
pub async fn get_attachment(
    Path((email_id, attachment_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, StatusCode> {
    let emails = state.emails.read().unwrap();
    let email = emails
        .iter()
        .find(|e| e.id == email_id)
        .ok_or(StatusCode::NOT_FOUND)?;

    let attachment = email
        .attachments
        .iter()
        .find(|a| a.id == attachment_id)
        .ok_or(StatusCode::NOT_FOUND)?;

    // Get the attachment data
    let data = attachment.data.as_ref().ok_or(StatusCode::NOT_FOUND)?;

    // Create response with appropriate headers
    let response = Response::builder()
        .header(header::CONTENT_TYPE, attachment.content_type.as_str())
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", attachment.filename),
        )
        .body(axum::body::Body::from(data.clone()))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(response)
}

/// Delete a specific email by ID
///
/// Returns 204 No Content if successful, or 404 if the email wasn't found
pub async fn delete_email(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> StatusCode {
    let mut emails = state.emails.write().unwrap();
    let initial_len = emails.len();
    emails.retain(|e| e.id != id);

    if emails.len() < initial_len {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

/// Delete all emails
///
/// Returns 204 No Content after clearing all emails
pub async fn delete_all_emails(State(state): State<Arc<AppState>>) -> StatusCode {
    let mut emails = state.emails.write().unwrap();
    emails.clear();
    StatusCode::NO_CONTENT
}

/// WebSocket handler for real-time updates
///
/// Upgrades the connection to a WebSocket and sends email updates in real-time
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

/// Handle WebSocket connection for real-time email updates
///
/// Sends all existing emails to the client and then streams new emails as they arrive
async fn handle_socket(socket: axum::extract::ws::WebSocket, state: Arc<AppState>) {
    // WebSocket implementation for real-time updates
    let (mut sender, _receiver) = socket.split();
    let mut rx = state.tx.subscribe();

    // Send initial emails - clone to avoid Send issues with RwLockReadGuard
    let emails_clone = {
        let emails_guard = state.emails.read().unwrap();
        emails_guard.clone()
    };

    for email in emails_clone.iter() {
        if let Ok(json) = serde_json::to_string(email) {
            if let Err(e) = sender
                .send(axum::extract::ws::Message::Text(Utf8Bytes::from(json)))
                .await
            {
                tracing::warn!("Failed to send WebSocket message: {}", e);
                return;
            }
        }
    }

    // Listen for new emails
    tokio::spawn(async move {
        while let Ok(email) = rx.recv().await {
            if let Ok(json) = serde_json::to_string(&email) {
                if let Err(e) = sender
                    .send(axum::extract::ws::Message::Text(Utf8Bytes::from(json)))
                    .await
                {
                    tracing::warn!("Failed to send WebSocket message: {}", e);
                    break;
                }
            }
        }
    });
}

/// Serve the main index.html page
///
/// Returns the HTML content of the main application page
pub async fn index() -> impl IntoResponse {
    // Get the index.html file from embedded assets
    match StaticAssets::get("html/index.html") {
        Some(content) => {
            let html = String::from_utf8_lossy(content.data.as_ref()).to_string();
            axum::response::Html(html)
        }
        None => {
            tracing::error!("Failed to find index.html in embedded assets");
            axum::response::Html(
                "<html><body><h1>Error loading page</h1></body></html>".to_string(),
            )
        }
    }
}

/// Serve static files (CSS, JS, images)
///
/// Returns the requested static file with appropriate content type headers
pub async fn static_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches("/static/");

    // Get the file from embedded assets
    match StaticAssets::get(path) {
        Some(content) => {
            // Guess the mime type
            let mime = from_path(path).first_or_octet_stream();

            // Create response with appropriate headers
            Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(axum::body::Body::from(content.data.to_vec()))
                .unwrap()
        }
        None => {
            tracing::error!("File not found in embedded assets: {}", path);
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(axum::body::Body::from("File not found".to_string()))
                .unwrap()
        }
    }
}

/// Start the HTTP server
///
/// Sets up routes for the API and static files, then starts the server on the specified port
pub async fn start_http_server(
    state: Arc<AppState>,
    port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create web server
    let app = Router::new()
        .route("/", get(index))
        .route("/api/emails", get(get_emails))
        .route("/api/emails", post(delete_all_emails))
        .route("/api/emails/{id}", get(get_email))
        .route("/api/emails/{id}", post(delete_email))
        .route(
            "/api/emails/{email_id}/attachments/{attachment_id}",
            get(get_attachment),
        )
        .route("/ws", get(ws_handler))
        // Serve static files from embedded assets
        .route("/static/{*path}", get(static_handler))
        .layer(CorsLayer::permissive())
        .with_state(state);

    // Start web server
    let web_addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    tracing::info!("Starting web server on {}", web_addr);
    let listener = tokio::net::TcpListener::bind(web_addr).await?;
    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{self, Body},
        http::{Request, StatusCode},
    };
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};
    use tokio::sync::broadcast;
    use tower::util::ServiceExt;
    use crate::models::Attachment;

    // Helper function to create a test AppState with sample emails
    fn create_test_state() -> Arc<AppState> {
        let (tx, _) = broadcast::channel(100);
        let emails = vec![
            Email {
                id: "test-email-1".to_string(),
                received_at: chrono::Utc::now(),
                from: "sender1@example.com".to_string(),
                to: vec!["recipient1@example.com".to_string()],
                subject: "Test Email 1".to_string(),
                text_body: Some("This is test email 1".to_string()),
                html_body: None,
                headers: HashMap::new(),
                attachments: Vec::new(),
            },
            Email {
                id: "test-email-2".to_string(),
                received_at: chrono::Utc::now(),
                from: "sender2@example.com".to_string(),
                to: vec!["recipient2@example.com".to_string()],
                subject: "Test Email 2".to_string(),
                text_body: Some("This is test email 2".to_string()),
                html_body: None,
                headers: HashMap::new(),
                attachments: vec![
                    Attachment {
                        id: "test-attachment-1".to_string(),
                        filename: "test.txt".to_string(),
                        content_type: "text/plain".to_string(),
                        size: 4,
                        data: Some(vec![116, 101, 115, 116]), // "test" in bytes
                    },
                ],
            },
        ];

        Arc::new(AppState {
            emails: RwLock::new(emails),
            tx,
        })
    }

    // Helper function to create a router with test state
    fn create_test_router() -> Router {
        let state = create_test_state();
        Router::new()
            .route("/api/emails", get(get_emails))
            .route("/api/emails", post(delete_all_emails))
            .route("/api/emails/{id}", get(get_email))
            .route("/api/emails/{id}", post(delete_email))
            .route("/api/emails/{email_id}/attachments/{attachment_id}", get(get_attachment))
            .route("/ws", get(ws_handler))
            .route("/", get(index))
            .route("/static/{path}", get(static_handler))
            .with_state(state)
    }

    #[tokio::test]
    async fn test_get_emails() {
        let app = create_test_router();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/emails")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let emails: Vec<Email> = serde_json::from_slice(&body).unwrap();

        assert_eq!(emails.len(), 2);
        assert_eq!(emails[0].id, "test-email-1");
        assert_eq!(emails[1].id, "test-email-2");
    }

    #[tokio::test]
    async fn test_get_email() {
        let app = create_test_router();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/emails/test-email-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let email: Email = serde_json::from_slice(&body).unwrap();

        assert_eq!(email.id, "test-email-1");
        assert_eq!(email.from, "sender1@example.com");
    }

    #[tokio::test]
    async fn test_get_email_not_found() {
        let app = create_test_router();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/emails/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_email() {
        let app = create_test_router();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/emails/test-email-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        // Verify the email was deleted
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/emails")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let emails: Vec<Email> = serde_json::from_slice(&body).unwrap();

        assert_eq!(emails.len(), 1);
        assert_eq!(emails[0].id, "test-email-2");
    }

    #[tokio::test]
    async fn test_delete_nonexistent_email() {
        let app = create_test_router();

        // Try to delete a non-existent email
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/emails/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return 404 Not Found
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_all_emails() {
        let app = create_test_router();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/emails")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        // Verify all emails were deleted
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/emails")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let emails: Vec<Email> = serde_json::from_slice(&body).unwrap();

        assert_eq!(emails.len(), 0);
    }

    #[tokio::test]
    async fn test_get_attachment() {
        let app = create_test_router();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/emails/test-email-2/attachments/test-attachment-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // Check content type header
        let content_type = response.headers().get("content-type").unwrap();
        assert_eq!(content_type, "text/plain");

        // Check content disposition header
        let content_disposition = response.headers().get("content-disposition").unwrap();
        assert_eq!(content_disposition, "attachment; filename=\"test.txt\"");

        // Check body content
        let body = body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(body, vec![116, 101, 115, 116]); // "test" in bytes
    }

    #[tokio::test]
    async fn test_get_attachment_not_found() {
        let app = create_test_router();

        // Test with non-existent email
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/emails/nonexistent/attachments/test-attachment-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // Test with non-existent attachment
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/emails/test-email-2/attachments/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_index() {
        let app = create_test_router();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // In a test environment, we can't access the embedded assets
        // The status should still be OK
        assert_eq!(response.status(), StatusCode::OK);

        // We don't need to check the exact content since it depends on the embedded assets
        // Just verify we can read the body
        let _body = body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_static_handler() {
        let app = create_test_router();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/static/test.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Since we can't access the embedded assets in tests, we expect a 404
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let body = body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&body);

        assert_eq!(text, "File not found");
    }
}
