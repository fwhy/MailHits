# MailHits

<p align="center">
    <img src="https://raw.githubusercontent.com/fwhy/MailHits/refs/heads/main/static/img/logo.png" alt="MailHits" width="200" height="200">
</p>

MailHits is a lightweight email testing tool that provides a simple SMTP server and web interface for capturing and viewing emails during development and testing.

## Features

- **SMTP Server**: Captures emails sent to any address on the configured port
- **Web Interface**: View captured emails in real-time
- **Email Parsing**: Parses email content including headers, text and HTML bodies
- **WebSocket Support**: Real-time updates when new emails arrive
- **Multiple View Formats**: View emails in HTML, plain text, or raw header format
- **No Configuration Needed**: Works out of the box with sensible defaults

## Installation

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (1.70.0 or later)

### Building from Source

1. Clone the repository:
   ```
   git clone https://github.com/fwhy/MailHits.git
   cd MailHits
   ```

2. Build the project:
   ```
   cargo build --release
   ```

3. The compiled binary will be available at `target/release/mailhits`

## Usage

### Starting the Server

Run MailHits with default settings:

```
./mailhits
```

This will start:
- SMTP server on port 1025
- Web interface on port 3000

### Command Line Options

```
./mailhits --help
```

Available options:
- `-s, --smtp-port <PORT>`: Set the SMTP server port (default: 1025)
- `-p, --http-port <PORT>`: Set the HTTP server port (default: 3000)

### Configuring Your Application

Configure your application to send emails to:
- SMTP server: `localhost`
- Port: `1025` (or your custom port)

No authentication is required.

### Viewing Emails

1. Open your web browser and navigate to `http://localhost:3000`
2. Send an email through your application
3. The email will appear in the MailHits web interface in real-time

## Development

MailHits is built with:
- [Tokio](https://tokio.rs/) for async runtime
- [Axum](https://github.com/tokio-rs/axum) for the web server
- [mail-parser](https://github.com/stalwartlabs/mail-parser) for email parsing

## Testing

MailHits includes a comprehensive test suite:

### Running Tests

```
cargo test
```

### Test Structure

- **Unit Tests**: Located within each module file (models.rs, smtp.rs, http.rs)
  - Test individual components in isolation
  - Verify correct behavior of email parsing, HTTP endpoints, etc.

### Test Coverage

The test suite covers:
- Email data structure creation and validation
- SMTP email processing
- HTTP API endpoints (get emails, get single email, delete emails)

## License

This project is open source and available under the [MIT License](LICENSE).
