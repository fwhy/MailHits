use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;
use tokio::sync::broadcast;

/// Data structures for email representation and application state

/// Represents an email message with all its components
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Email {
    /// Unique identifier for the email
    pub id: String,
    /// Timestamp when the email was received
    pub received_at: DateTime<Utc>,
    /// Email sender address
    pub from: String,
    /// List of recipient email addresses
    pub to: Vec<String>,
    /// Email subject line
    pub subject: String,
    /// Plain text version of the email body (if available)
    pub text_body: Option<String>,
    /// HTML version of the email body (if available)
    pub html_body: Option<String>,
    /// Map of email headers
    pub headers: HashMap<String, String>,
    /// List of email attachments
    pub attachments: Vec<Attachment>,
}

/// Represents an email attachment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    /// Unique identifier for the attachment
    pub id: String,
    /// Original filename of the attachment
    pub filename: String,
    /// MIME content type of the attachment
    pub content_type: String,
    /// Size of the attachment in bytes
    pub size: usize,
    /// Binary data of the attachment (not serialized to JSON)
    #[serde(skip_serializing)]
    pub data: Option<Vec<u8>>,
}

/// Application state shared between SMTP and HTTP servers
pub struct AppState {
    /// Thread-safe storage for captured emails
    pub emails: RwLock<Vec<Email>>,
    /// Broadcast channel for real-time notifications about new emails
    pub tx: broadcast::Sender<Email>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn test_email_creation() {
        let email = Email {
            id: Uuid::new_v4().to_string(),
            received_at: Utc::now(),
            from: "sender@example.com".to_string(),
            to: vec!["recipient@example.com".to_string()],
            subject: "Test Subject".to_string(),
            text_body: Some("This is a test email".to_string()),
            html_body: Some("<p>This is a test email</p>".to_string()),
            headers: HashMap::new(),
            attachments: Vec::new(),
        };

        assert_eq!(email.from, "sender@example.com");
        assert_eq!(email.to[0], "recipient@example.com");
        assert_eq!(email.subject, "Test Subject");
        assert_eq!(email.text_body, Some("This is a test email".to_string()));
        assert_eq!(
            email.html_body,
            Some("<p>This is a test email</p>".to_string())
        );
        assert!(email.attachments.is_empty());
    }

    #[test]
    fn test_attachment_creation() {
        let attachment = Attachment {
            id: Uuid::new_v4().to_string(),
            filename: "test.txt".to_string(),
            content_type: "text/plain".to_string(),
            size: 1024,
            data: Some(vec![0, 1, 2, 3]),
        };

        assert_eq!(attachment.filename, "test.txt");
        assert_eq!(attachment.content_type, "text/plain");
        assert_eq!(attachment.size, 1024);
    }
}
