use chrono::Utc;
use mail_parser::{HeaderValue, MessageParser, MimeHeaders};
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tracing::{info, warn};
use uuid::Uuid;

use crate::models::{AppState, Attachment, Email};

/// SMTP server implementation for capturing emails

/// Handle an individual SMTP client connection
///
/// Processes SMTP commands from the client and captures any emails sent
pub async fn handle_smtp_client(
    mut stream: TcpStream,
    _addr: SocketAddr,
    state: Arc<AppState>,
) -> io::Result<()> {
    let (reader, mut writer) = stream.split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    // Send greeting
    writer
        .write_all(b"220 MailHits SMTP Server ready\r\n")
        .await?;

    // SMTP state
    let mut mail_from = String::new();
    let mut rcpt_to = Vec::new();
    let mut in_data = false;
    let mut data_buffer = Vec::new();

    // Process commands
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            break; // Connection closed
        }

        info!("SMTP << {}", line.trim());

        if in_data {
            // In DATA mode, collect lines until we see a lone "."
            if line.trim() == "." {
                in_data = false;

                // Process the collected email data
                if let Err(e) = process_email(
                    &data_buffer,
                    mail_from.clone(),
                    rcpt_to.clone(),
                    state.clone(),
                )
                .await
                {
                    warn!("Failed to process email: {}", e);
                    writer.write_all(b"554 Transaction failed\r\n").await?;
                } else {
                    writer.write_all(b"250 OK: Message accepted\r\n").await?;
                }

                // Reset state for next email
                mail_from.clear();
                rcpt_to.clear();
                data_buffer.clear();
            } else {
                // Collect the data line (removing the leading dot if line starts with ..)
                let line_content = if line.starts_with("..") {
                    line[1..].to_string()
                } else {
                    line.clone()
                };
                data_buffer.extend_from_slice(line_content.as_bytes());
            }
            continue;
        }

        // Parse command
        let parts: Vec<&str> = line.trim().splitn(2, ' ').collect();
        let command = parts[0].to_uppercase();

        match command.as_str() {
            "HELO" | "EHLO" => {
                let domain = if parts.len() > 1 { parts[1] } else { "unknown" };
                info!("HELO from {}", domain);
                writer.write_all(b"250 MailHits\r\n").await?;
            }
            "MAIL" => {
                if let Some(from_part) = parts.get(1) {
                    if let Some(from) = from_part.strip_prefix("FROM:") {
                        mail_from = from.trim().to_string();
                        mail_from.retain(|c| c != '<' && c != '>');
                        info!("MAIL FROM: {}", mail_from);
                        writer.write_all(b"250 OK\r\n").await?;
                    } else {
                        writer
                            .write_all(b"501 Syntax error in parameters\r\n")
                            .await?;
                    }
                } else {
                    writer
                        .write_all(b"501 Syntax error in parameters\r\n")
                        .await?;
                }
            }
            "RCPT" => {
                if let Some(to_part) = parts.get(1) {
                    if let Some(to) = to_part.strip_prefix("TO:") {
                        let mut to = to.trim().to_string();
                        to.retain(|c| c != '<' && c != '>');
                        rcpt_to.push(to.clone());
                        info!("RCPT TO: {}", to);
                        writer.write_all(b"250 OK\r\n").await?;
                    } else {
                        writer
                            .write_all(b"501 Syntax error in parameters\r\n")
                            .await?;
                    }
                } else {
                    writer
                        .write_all(b"501 Syntax error in parameters\r\n")
                        .await?;
                }
            }
            "DATA" => {
                if mail_from.is_empty() || rcpt_to.is_empty() {
                    writer
                        .write_all(b"503 Bad sequence of commands\r\n")
                        .await?;
                } else {
                    writer
                        .write_all(b"354 Start mail input; end with <CRLF>.<CRLF>\r\n")
                        .await?;
                    in_data = true;
                    data_buffer.clear();
                }
            }
            "RSET" => {
                mail_from.clear();
                rcpt_to.clear();
                data_buffer.clear();
                in_data = false;
                writer.write_all(b"250 OK\r\n").await?;
            }
            "NOOP" => {
                writer.write_all(b"250 OK\r\n").await?;
            }
            "QUIT" => {
                writer
                    .write_all(b"221 MailHits closing connection\r\n")
                    .await?;
                break;
            }
            _ => {
                writer.write_all(b"500 Command not recognized\r\n").await?;
            }
        }
    }

    Ok(())
}

/// Process an email received via SMTP
///
/// Parses the raw email data, extracts headers, body parts, and attachments,
/// then stores the email in the application state and broadcasts it to WebSocket clients
async fn process_email(
    data: &[u8],
    from: String,
    to: Vec<String>,
    state: Arc<AppState>,
) -> io::Result<()> {
    // Try to parse the email data
    let email_str = String::from_utf8_lossy(data);

    // Try to parse with mail-parser
    let parsed = match MessageParser::default().parse(data) {
        Some(parsed) => parsed,
        None => {
            // Fallback to simple parsing
            return process_email_simple(&email_str, from, to, state).await;
        }
    };

    // Extract headers - simplified approach
    let mut headers = HashMap::new();
    for header in parsed.headers() {
        let name = header.name().to_string();
        // Convert header value to string - just use a simple representation
        let value = match header.value() {
            HeaderValue::Address(address) => address
                .iter()
                .map(|addr| {
                    format!(
                        "{}&lt;{}&gt;",
                        addr.clone().name.unwrap_or_default().to_string(),
                        addr.clone().address.unwrap().to_string()
                    )
                    .trim()
                    .to_string()
                })
                .collect::<Vec<String>>()
                .join(", "),
            HeaderValue::Text(text) => text.to_string(),
            HeaderValue::ContentType(content_type) => format!(
                "{}/{}; {}",
                content_type.clone().c_type,
                content_type.clone().c_subtype.unwrap_or_default(),
                content_type
                    .clone()
                    .attributes
                    .unwrap_or_default()
                    .iter()
                    .map(|attr| format!("{}={}", attr.name, attr.value))
                    .collect::<Vec<String>>()
                    .join("; ")
            ),
            HeaderValue::DateTime(date_time) => date_time.to_string(),
            _ => format!("{:?}", header.value()),
        };
        // let value = format!("{:?}", header.value());
        headers.insert(name, value);
    }

    // Extract subject
    let subject = parsed.subject().unwrap_or("No Subject").to_string();

    // Extract body parts
    let mut text_body = None;
    let mut html_body = None;

    // Get text body from parts
    if parsed.text_body_count() > 0 {
        if let Some(part) = parsed.text_bodies().next() {
            text_body = Some(part.to_string());
        }
    }

    // Get HTML body from parts
    if parsed.html_body_count() > 0 {
        if let Some(part) = parsed.html_bodies().next() {
            html_body = Some(part.to_string());
        }
    }

    // Extract attachments
    let mut attachments = Vec::new();
    for attachment in parsed.attachments() {
        let id = Uuid::new_v4().to_string();
        let filename = attachment
            .attachment_name()
            .unwrap_or("unnamed")
            .to_string();
        let content_type = if let Some(ct) = attachment.content_type() {
            format!(
                "{}/{}",
                ct.c_type,
                ct.c_subtype
                    .as_ref()
                    .map(|s| s.as_ref())
                    .unwrap_or_default()
            )
        } else {
            "application/octet-stream".to_string()
        };
        let data = attachment.contents().to_vec();
        let size = data.len();

        attachments.push(Attachment {
            id,
            filename,
            content_type,
            size,
            data: Some(data),
        });
    }

    // Create email object
    let email = Email {
        id: Uuid::new_v4().to_string(),
        received_at: Utc::now(),
        from,
        to,
        subject,
        text_body,
        html_body,
        headers,
        attachments,
    };

    // Store email
    {
        let mut emails = state.emails.write().unwrap();
        emails.push(email.clone());
    }

    // Broadcast to websocket clients
    let _ = state.tx.send(email);

    Ok(())
}

/// Process an email using a simple parser when the main parser fails
///
/// Fallback method for parsing emails that can't be parsed by the mail-parser library.
/// Uses a simpler line-by-line approach to extract headers and body.
async fn process_email_simple(
    email_str: &str,
    from: String,
    to: Vec<String>,
    state: Arc<AppState>,
) -> io::Result<()> {
    // Extract headers and body
    let mut headers = HashMap::new();
    let mut body_parts = Vec::new();

    let mut lines = email_str.lines();
    let mut in_headers = true;
    let mut current_header = String::new();

    // Simple parser for email format
    while let Some(line) = lines.next() {
        if in_headers {
            if line.is_empty() {
                in_headers = false;
                continue;
            }

            if line.starts_with(' ') || line.starts_with('\t') {
                // Continuation of previous header
                current_header.push(' ');
                current_header.push_str(line.trim());
            } else {
                // New header
                if !current_header.is_empty() {
                    if let Some(colon_pos) = current_header.find(':') {
                        let key = current_header[..colon_pos].trim().to_string();
                        let value = current_header[colon_pos + 1..].trim().to_string();
                        headers.insert(key, value);
                    }
                }
                current_header = line.to_string();
            }
        } else {
            // Body
            body_parts.push(line.to_string());
        }
    }

    // // Process the last header if any
    // if !current_header.is_empty() && in_headers {
    //     if let Some(colon_pos) = current_header.find(':') {
    //         let key = current_header[..colon_pos].trim().to_string();
    //         let value = current_header[colon_pos + 1..].trim().to_string();
    //         headers.insert(key, value);
    //     }
    // }

    // Extract subject
    let subject = headers
        .get("Subject")
        .cloned()
        .unwrap_or_else(|| "No Subject".to_string());

    // Simple content type detection
    let content_type = headers
        .get("Content-Type")
        .cloned()
        .unwrap_or_else(|| "text/plain".to_string());

    let body = body_parts.join("\n");

    let (text_body, html_body) = if content_type.contains("text/html") {
        (None, Some(body))
    } else {
        (Some(body), None)
    };

    // Create email object
    let email = Email {
        id: Uuid::new_v4().to_string(),
        received_at: Utc::now(),
        from,
        to,
        subject,
        text_body,
        html_body,
        headers,
        attachments: Vec::new(), // Simple implementation without attachment parsing
    };

    // Store email
    {
        let mut emails = state.emails.write().unwrap();
        emails.push(email.clone());
    }

    // Broadcast to websocket clients
    let _ = state.tx.send(email);

    Ok(())
}

/// Start the SMTP server
///
/// Binds to the specified port and listens for incoming SMTP connections.
/// Each connection is handled in a separate task.
pub async fn start_smtp_server(state: Arc<AppState>, port: u16) {
    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], port));

    info!("Starting SMTP server on {}", smtp_addr);

    // Create a TCP listener for the SMTP server
    match TcpListener::bind(smtp_addr).await {
        Ok(listener) => {
            info!("SMTP server listening on {}", smtp_addr);

            // Accept connections and handle them
            loop {
                match listener.accept().await {
                    Ok((stream, addr)) => {
                        let state_clone = state.clone();

                        // Handle the connection
                        tokio::spawn(async move {
                            if let Err(e) = handle_smtp_client(stream, addr, state_clone).await {
                                warn!("SMTP session error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        warn!("Failed to accept SMTP connection: {}", e);
                    }
                }
            }
        }
        Err(e) => {
            warn!("Failed to bind SMTP server: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::RwLock;
    use tokio::sync::broadcast;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use std::time::Duration;

    // Helper function to create a test AppState
    fn create_test_state() -> Arc<AppState> {
        let (tx, _) = broadcast::channel(100);
        Arc::new(AppState {
            emails: RwLock::new(Vec::<Email>::new()),
            tx,
        })
    }

    #[tokio::test]
    async fn test_process_email_simple() {
        let state = create_test_state();
        let from = "sender@example.com".to_string();
        let to = vec!["recipient@example.com".to_string()];

        let email_data = "From: sender@example.com\r\n\
                          To: recipient@example.com\r\n\
                          Subject: Test Email\r\n\
                          Content-Type: text/plain\r\n\
                          \r\n\
                          This is a test email body.";

        let result = process_email_simple(email_data, from, to.clone(), state.clone()).await;
        assert!(result.is_ok());

        // Verify the email was stored
        let emails = state.emails.read().unwrap();
        assert_eq!(emails.len(), 1);

        let email = &emails[0];
        assert_eq!(email.from, "sender@example.com");
        assert_eq!(email.to, to);
        assert_eq!(email.subject, "Test Email");
        assert_eq!(
            email.text_body,
            Some("This is a test email body.".to_string())
        );
    }

    #[tokio::test]
    async fn test_process_email() {
        let state = create_test_state();
        let from = "sender@example.com".to_string();
        let to = vec!["recipient@example.com".to_string()];

        let email_data = "From: sender@example.com\r\n\
                          To: recipient@example.com\r\n\
                          Subject: Test Email\r\n\
                          Content-Type: text/plain\r\n\
                          \r\n\
                          This is a test email body."
            .as_bytes()
            .to_vec();

        let result = process_email(&email_data, from, to.clone(), state.clone()).await;
        assert!(result.is_ok());

        // Verify the email was stored
        let emails = state.emails.read().unwrap();
        assert_eq!(emails.len(), 1);

        let email = &emails[0];
        assert_eq!(email.from, "sender@example.com");
        assert_eq!(email.to, to);
        assert!(email.subject.contains("Test Email"));
    }

    #[tokio::test]
    async fn test_process_email_with_attachment() {
        let state = create_test_state();
        let from = "sender@example.com".to_string();
        let to = vec!["recipient@example.com".to_string()];

        // Create a multipart email with an attachment
        let email_data = "From: sender@example.com\r\n\
                          To: recipient@example.com\r\n\
                          Subject: Test Email with Attachment\r\n\
                          Content-Type: multipart/mixed; boundary=boundary\r\n\
                          \r\n\
                          --boundary\r\n\
                          Content-Type: text/plain\r\n\
                          \r\n\
                          This is a test email body.\r\n\
                          --boundary\r\n\
                          Content-Type: text/plain; name=\"test.txt\"\r\n\
                          Content-Disposition: attachment; filename=\"test.txt\"\r\n\
                          \r\n\
                          This is a test attachment.\r\n\
                          --boundary--"
            .as_bytes()
            .to_vec();

        let result = process_email(&email_data, from, to.clone(), state.clone()).await;
        assert!(result.is_ok());

        // Verify the email was stored
        let emails = state.emails.read().unwrap();
        assert_eq!(emails.len(), 1);

        let email = &emails[0];
        assert_eq!(email.from, "sender@example.com");
        assert_eq!(email.to, to);
        assert!(email.subject.contains("Test Email with Attachment"));

        // Check if the attachment was processed
        assert!(!email.attachments.is_empty());
        if !email.attachments.is_empty() {
            let attachment = &email.attachments[0];
            assert_eq!(attachment.filename, "test.txt");
            assert!(attachment.content_type.contains("text/plain"));
            assert!(attachment.size > 0);
            assert!(attachment.data.is_some());
        }
    }

    #[tokio::test]
    async fn test_process_email_with_html() {
        let state = create_test_state();
        let from = "sender@example.com".to_string();
        let to = vec!["recipient@example.com".to_string()];

        // Create an email with HTML content
        let email_data = "From: sender@example.com\r\n\
                          To: recipient@example.com\r\n\
                          Subject: Test Email with HTML\r\n\
                          Content-Type: multipart/alternative; boundary=boundary\r\n\
                          \r\n\
                          --boundary\r\n\
                          Content-Type: text/plain\r\n\
                          \r\n\
                          This is a test email body.\r\n\
                          --boundary\r\n\
                          Content-Type: text/html\r\n\
                          \r\n\
                          <html><body><p>This is a test email body.</p></body></html>\r\n\
                          --boundary--"
            .as_bytes()
            .to_vec();

        let result = process_email(&email_data, from, to.clone(), state.clone()).await;
        assert!(result.is_ok());

        // Verify the email was stored
        let emails = state.emails.read().unwrap();
        assert_eq!(emails.len(), 1);

        let email = &emails[0];
        assert_eq!(email.from, "sender@example.com");
        assert_eq!(email.to, to);
        assert!(email.subject.contains("Test Email with HTML"));

        // Check if both text and HTML bodies were processed
        assert!(email.text_body.is_some());
        assert!(email.html_body.is_some());
        if let Some(text_body) = &email.text_body {
            assert!(text_body.contains("This is a test email body"));
        }
        if let Some(html_body) = &email.html_body {
            assert!(html_body.contains("<html><body><p>This is a test email body.</p></body></html>"));
        }
    }

    #[tokio::test]
    async fn test_handle_smtp_client() {
        let state = create_test_state();

        // Create a TCP listener for the test
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Spawn a task to handle the client connection
        let state_clone = state.clone();
        let handle_task = tokio::spawn(async move {
            let (stream, client_addr) = listener.accept().await.unwrap();
            handle_smtp_client(stream, client_addr, state_clone).await
        });

        // Connect to the server
        let mut stream = TcpStream::connect(addr).await.unwrap();

        // Read the greeting
        let mut buffer = [0; 1024];
        let n = stream.read(&mut buffer).await.unwrap();
        let greeting = String::from_utf8_lossy(&buffer[..n]);
        assert!(greeting.contains("220 MailHits SMTP Server ready"));

        // Send EHLO
        stream.write_all(b"EHLO example.com\r\n").await.unwrap();
        let n = stream.read(&mut buffer).await.unwrap();
        let response = String::from_utf8_lossy(&buffer[..n]);
        assert!(response.contains("250 MailHits"));

        // Send MAIL FROM
        stream.write_all(b"MAIL FROM:<sender@example.com>\r\n").await.unwrap();
        let n = stream.read(&mut buffer).await.unwrap();
        let response = String::from_utf8_lossy(&buffer[..n]);
        assert!(response.contains("250 OK"));

        // Send RCPT TO
        stream.write_all(b"RCPT TO:<recipient@example.com>\r\n").await.unwrap();
        let n = stream.read(&mut buffer).await.unwrap();
        let response = String::from_utf8_lossy(&buffer[..n]);
        assert!(response.contains("250 OK"));

        // Send DATA command
        stream.write_all(b"DATA\r\n").await.unwrap();
        let n = stream.read(&mut buffer).await.unwrap();
        let response = String::from_utf8_lossy(&buffer[..n]);
        assert!(response.contains("354 Start mail input"));

        // Send email content
        stream.write_all(b"From: sender@example.com\r\n").await.unwrap();
        stream.write_all(b"To: recipient@example.com\r\n").await.unwrap();
        stream.write_all(b"Subject: Test Email\r\n").await.unwrap();
        stream.write_all(b"\r\n").await.unwrap();
        stream.write_all(b"This is a test email body.\r\n").await.unwrap();
        stream.write_all(b".\r\n").await.unwrap();

        let n = stream.read(&mut buffer).await.unwrap();
        let response = String::from_utf8_lossy(&buffer[..n]);
        assert!(response.contains("250 OK: Message accepted"));

        // Send QUIT
        stream.write_all(b"QUIT\r\n").await.unwrap();
        let n = stream.read(&mut buffer).await.unwrap();
        let response = String::from_utf8_lossy(&buffer[..n]);
        assert!(response.contains("221 MailHits closing connection"));

        // Wait for the handler to complete
        handle_task.await.unwrap().unwrap();

        // Verify the email was stored
        let emails = state.emails.read().unwrap();
        assert_eq!(emails.len(), 1);

        let email = &emails[0];
        assert_eq!(email.from, "sender@example.com");
        assert_eq!(email.to, vec!["recipient@example.com"]);
        assert!(email.subject.contains("Test Email"));
    }

    #[tokio::test]
    async fn test_handle_smtp_client_commands() {
        let state = create_test_state();

        // Create a TCP listener for the test
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Spawn a task to handle the client connection
        let state_clone = state.clone();
        let handle_task = tokio::spawn(async move {
            let (stream, client_addr) = listener.accept().await.unwrap();
            handle_smtp_client(stream, client_addr, state_clone).await
        });

        // Connect to the server
        let mut stream = TcpStream::connect(addr).await.unwrap();

        // Read the greeting
        let mut buffer = [0; 1024];
        let n = stream.read(&mut buffer).await.unwrap();
        let greeting = String::from_utf8_lossy(&buffer[..n]);
        assert!(greeting.contains("220 MailHits SMTP Server ready"));

        // Test HELO
        stream.write_all(b"HELO example.com\r\n").await.unwrap();
        let n = stream.read(&mut buffer).await.unwrap();
        let response = String::from_utf8_lossy(&buffer[..n]);
        assert!(response.contains("250 MailHits"));

        // Test MAIL FROM with syntax error
        stream.write_all(b"MAIL\r\n").await.unwrap();
        let n = stream.read(&mut buffer).await.unwrap();
        let response = String::from_utf8_lossy(&buffer[..n]);
        assert!(response.contains("501 Syntax error"));

        // Test RCPT TO with syntax error
        stream.write_all(b"RCPT\r\n").await.unwrap();
        let n = stream.read(&mut buffer).await.unwrap();
        let response = String::from_utf8_lossy(&buffer[..n]);
        assert!(response.contains("501 Syntax error"));

        // Test DATA without MAIL FROM and RCPT TO
        stream.write_all(b"DATA\r\n").await.unwrap();
        let n = stream.read(&mut buffer).await.unwrap();
        let response = String::from_utf8_lossy(&buffer[..n]);
        assert!(response.contains("503 Bad sequence"));

        // Test NOOP
        stream.write_all(b"NOOP\r\n").await.unwrap();
        let n = stream.read(&mut buffer).await.unwrap();
        let response = String::from_utf8_lossy(&buffer[..n]);
        assert!(response.contains("250 OK"));

        // Test RSET
        stream.write_all(b"RSET\r\n").await.unwrap();
        let n = stream.read(&mut buffer).await.unwrap();
        let response = String::from_utf8_lossy(&buffer[..n]);
        assert!(response.contains("250 OK"));

        // Test unknown command
        stream.write_all(b"UNKNOWN\r\n").await.unwrap();
        let n = stream.read(&mut buffer).await.unwrap();
        let response = String::from_utf8_lossy(&buffer[..n]);
        assert!(response.contains("500 Command not recognized"));

        // Send QUIT
        stream.write_all(b"QUIT\r\n").await.unwrap();
        let n = stream.read(&mut buffer).await.unwrap();
        let response = String::from_utf8_lossy(&buffer[..n]);
        assert!(response.contains("221 MailHits closing connection"));

        // Wait for the handler to complete
        handle_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn test_start_smtp_server() {
        let state = create_test_state();

        // Find an available port
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        // Start the SMTP server in a separate task
        let state_clone = state.clone();
        let server_task = tokio::spawn(async move {
            start_smtp_server(state_clone, port).await;
        });

        // Give the server a moment to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Try to connect to the server
        let result = TcpStream::connect(format!("127.0.0.1:{}", port)).await;
        assert!(result.is_ok());

        // We can't easily stop the server, so we'll just cancel the task
        server_task.abort();
    }
}
