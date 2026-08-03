//! Send-to-Kindle via Amazon's **official personal-documents email flow**.
//!
//! This is the legitimate, ToS-compliant path: the user emails their **own,
//! DRM-free** EPUB to their `@kindle.com` address, from an address they have
//! registered as an *Approved Personal Document Email* in their Amazon account,
//! over **their own** SMTP account. Amazon then delivers it to their Kindle
//! library. EPUB is now natively supported by Send to Kindle, so no MOBI/AZW
//! conversion and no `convert` subject keyword are needed.
//!
//! ## What Libro will and won't do (legal posture — mirrors `ARCHITECTURE.md`)
//! * **Only the user's own DRM-free ebooks.** The bytes come from the Local
//!   Files connector (files the user already has on disk). No DRM handling, ever.
//! * **User-owned SMTP credentials**, held in the user's local, encrypted config.
//! * **User-owned addresses.** The `from` must be one the user registered with
//!   Amazon; the `to` must be an `@kindle.com` address.
//! * **No Amazon private API, no reverse-engineering, no scraping.** Send-to-Kindle
//!   is official-email-only.
//!
//! ## Design (consistent with the rest of core)
//! The **message construction** ([`build_kindle_message`]) is a pure, unit-tested
//! helper kept SEPARATE from the SMTP transport, exactly like the `map_*` mapping
//! helpers on the connectors. The transport itself sits behind the small
//! [`KindleSender`] seam so the orchestration logic ([`send_epub_to_kindle`]) is
//! testable with a fake sender and **never touches the network in tests**.
//!
//! Unlike the background progress syncs (which swallow errors), Send-to-Kindle is
//! a **user-initiated action**, so [`send_epub_to_kindle`] returns a typed
//! [`SendOutcome`] the UI surfaces as success or a clear failure.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use lettre::message::{header::ContentType, Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

/// The MIME type Amazon expects for an EPUB personal document.
pub const EPUB_MIME: &str = "application/epub+zip";

/// Amazon's per-email attachment size cap for personal documents (~50 MB). We
/// enforce this locally so an oversized book fails fast with a typed error
/// instead of being rejected after a slow upload.
pub const MAX_ATTACHMENT_BYTES: usize = 50 * 1024 * 1024;

/// Local, user-owned SMTP + Kindle-delivery settings.
///
/// Stored inside the encrypted [`crate::config::AppConfig`]. `smtp_password` is a
/// **secret** — the UI masks it and never echoes it back, and it is only ever
/// held decrypted in memory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KindleConfig {
    /// SMTP submission host, e.g. `smtp.gmail.com`.
    #[serde(default)]
    pub smtp_host: String,
    /// SMTP submission port (typically 587 for STARTTLS or 465 for implicit TLS).
    #[serde(default)]
    pub smtp_port: u16,
    /// SMTP username (often the same as `from_address`).
    #[serde(default)]
    pub smtp_username: String,
    /// SMTP password / app-password. Secret.
    #[serde(default)]
    pub smtp_password: String,
    /// The **Approved Personal Document Email** the user registered with Amazon.
    #[serde(default)]
    pub from_address: String,
    /// The destination `@kindle.com` address.
    #[serde(default)]
    pub to_address: String,
}

/// Why a [`KindleConfig`] is not usable, from the pure validators.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KindleConfigError {
    #[error("SMTP host is required")]
    MissingHost,
    #[error("SMTP port is invalid")]
    InvalidPort,
    #[error("SMTP username is required")]
    MissingUsername,
    #[error("SMTP password is required")]
    MissingPassword,
    #[error("a sender (from) address is required")]
    MissingFrom,
    #[error("the destination must be an @kindle.com address")]
    NotKindleAddress,
}

/// The typed result of a send attempt, returned to the frontend.
///
/// Serialized with a `status` tag so the UI can branch (`sent`, `not_configured`,
/// `too_large`, `not_an_epub`, `send_failed`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SendOutcome {
    /// The message was accepted by the SMTP server.
    Sent,
    /// Send-to-Kindle isn't configured (or the config is invalid) — nothing sent.
    NotConfigured,
    /// The attachment exceeds [`MAX_ATTACHMENT_BYTES`]; not sent.
    TooLarge { size: usize, limit: usize },
    /// The file isn't an EPUB (Libro only sends DRM-free EPUBs it holds locally).
    NotAnEpub,
    /// The SMTP transport reported an error.
    SendFailed { reason: String },
}

/// True iff `addr` is an `@kindle.com` address (case-insensitive).
pub fn is_kindle_address(addr: &str) -> bool {
    let addr = addr.trim().to_ascii_lowercase();
    // A bare "@kindle.com" (empty local part) is not a valid address.
    matches!(addr.strip_suffix("@kindle.com"), Some(local) if !local.is_empty())
}

/// True iff `name` looks like an EPUB file (case-insensitive `.epub`).
pub fn is_epub_filename(name: &str) -> bool {
    name.trim().to_ascii_lowercase().ends_with(".epub")
}

/// Validate a [`KindleConfig`]. Pure; no I/O. Returns the **first** problem so
/// the UI can show a specific message.
pub fn validate_config(cfg: &KindleConfig) -> Result<(), KindleConfigError> {
    if cfg.smtp_host.trim().is_empty() {
        return Err(KindleConfigError::MissingHost);
    }
    if cfg.smtp_port == 0 {
        return Err(KindleConfigError::InvalidPort);
    }
    if cfg.smtp_username.trim().is_empty() {
        return Err(KindleConfigError::MissingUsername);
    }
    if cfg.smtp_password.is_empty() {
        return Err(KindleConfigError::MissingPassword);
    }
    if cfg.from_address.trim().is_empty() {
        return Err(KindleConfigError::MissingFrom);
    }
    if !is_kindle_address(&cfg.to_address) {
        return Err(KindleConfigError::NotKindleAddress);
    }
    Ok(())
}

/// Whether Send-to-Kindle is fully configured (used to gate the UI/command).
pub fn is_configured(cfg: &KindleConfig) -> bool {
    validate_config(cfg).is_ok()
}

/// Build the MIME email carrying `bytes` as an `application/epub+zip` attachment.
///
/// **Pure** message construction — no network. Kept separate from the transport
/// so the multipart structure (headers, attachment MIME, base64 body) can be
/// asserted in unit tests. `from`/`to` must parse as mailbox addresses.
pub fn build_kindle_message(
    from: &str,
    to: &str,
    subject: &str,
    filename: &str,
    bytes: &[u8],
) -> Result<Message, String> {
    let from: Mailbox = from
        .trim()
        .parse()
        .map_err(|e| format!("invalid from address: {e}"))?;
    let to: Mailbox = to
        .trim()
        .parse()
        .map_err(|e| format!("invalid to address: {e}"))?;

    let content_type =
        ContentType::parse(EPUB_MIME).map_err(|e| format!("invalid content type: {e}"))?;
    let attachment = Attachment::new(filename.to_string()).body(bytes.to_vec(), content_type);

    // A short text part keeps some mail servers happy about an all-attachment
    // message; Amazon only cares about the EPUB attachment.
    let body = SinglePart::plain(format!("Sent to Kindle by Libro: {filename}"));

    Message::builder()
        .from(from)
        .to(to)
        .subject(subject.to_string())
        .multipart(MultiPart::mixed().singlepart(body).singlepart(attachment))
        .map_err(|e| format!("failed to build message: {e}"))
}

/// The transport seam. The real implementation talks SMTP over rustls; tests
/// inject a fake to assert the orchestration logic without a network.
#[async_trait]
pub trait KindleSender: Send + Sync {
    /// Send an already-built message. Returns a human-readable error on failure.
    async fn send(&self, message: &Message) -> Result<(), String>;
}

/// The real SMTP transport, built from a [`KindleConfig`].
///
/// Uses rustls (no native-tls/OpenSSL) so it builds under the
/// `x86_64-pc-windows-gnu` toolchain, consistent with our `reqwest` setup. Port
/// 465 uses implicit TLS ([`AsyncSmtpTransport::relay`]); any other port uses
/// STARTTLS ([`AsyncSmtpTransport::starttls_relay`], the common 587 submission
/// path).
pub struct SmtpKindleSender {
    transport: AsyncSmtpTransport<Tokio1Executor>,
}

impl SmtpKindleSender {
    /// Construct the SMTP transport from user-owned credentials. Does not connect
    /// yet — the connection opens on the first [`KindleSender::send`].
    pub fn from_config(cfg: &KindleConfig) -> Result<Self, String> {
        let creds = Credentials::new(cfg.smtp_username.clone(), cfg.smtp_password.clone());
        let builder = if cfg.smtp_port == 465 {
            AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.smtp_host)
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.smtp_host)
        }
        .map_err(|e| format!("invalid SMTP configuration: {e}"))?;

        let transport = builder.port(cfg.smtp_port).credentials(creds).build();
        Ok(Self { transport })
    }
}

#[async_trait]
impl KindleSender for SmtpKindleSender {
    async fn send(&self, message: &Message) -> Result<(), String> {
        self.transport
            .send(message.clone())
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Orchestrate a Send-to-Kindle: validate config → guard EPUB + size → build the
/// message → hand it to the transport. Pure of I/O except the injected
/// `sender.send`, so it is fully unit-testable with a fake sender.
///
/// The caller resolves `bytes` from the user's local EPUB and passes a `subject`
/// (typically the book title). Returns a typed [`SendOutcome`] — never panics.
pub async fn send_epub_to_kindle(
    cfg: &KindleConfig,
    subject: &str,
    filename: &str,
    bytes: &[u8],
    sender: &dyn KindleSender,
) -> SendOutcome {
    if !is_configured(cfg) {
        return SendOutcome::NotConfigured;
    }
    if !is_epub_filename(filename) {
        return SendOutcome::NotAnEpub;
    }
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return SendOutcome::TooLarge {
            size: bytes.len(),
            limit: MAX_ATTACHMENT_BYTES,
        };
    }

    let message =
        match build_kindle_message(&cfg.from_address, &cfg.to_address, subject, filename, bytes) {
            Ok(m) => m,
            Err(reason) => return SendOutcome::SendFailed { reason },
        };

    match sender.send(&message).await {
        Ok(()) => SendOutcome::Sent,
        Err(reason) => SendOutcome::SendFailed { reason },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn valid_config() -> KindleConfig {
        KindleConfig {
            smtp_host: "smtp.example.com".into(),
            smtp_port: 587,
            smtp_username: "user@example.com".into(),
            smtp_password: "app-password".into(),
            from_address: "user@example.com".into(),
            to_address: "reader@kindle.com".into(),
        }
    }

    /// Fake transport that records how many messages it was asked to send and the
    /// raw bytes of the last one — no network.
    #[derive(Default)]
    struct FakeSender {
        calls: Mutex<usize>,
        last: Mutex<Option<Vec<u8>>>,
        fail_with: Option<String>,
    }

    #[async_trait]
    impl KindleSender for FakeSender {
        async fn send(&self, message: &Message) -> Result<(), String> {
            *self.calls.lock().unwrap() += 1;
            *self.last.lock().unwrap() = Some(message.formatted());
            match &self.fail_with {
                Some(e) => Err(e.clone()),
                None => Ok(()),
            }
        }
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        // The default test suite is synchronous; drive one future to completion
        // with a minimal current-thread runtime (tokio is a dev-dependency).
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    // ---- address + filename predicates ------------------------------------

    #[test]
    fn kindle_address_predicate() {
        assert!(is_kindle_address("reader@kindle.com"));
        assert!(is_kindle_address("Reader@Kindle.Com")); // case-insensitive
        assert!(!is_kindle_address("reader@gmail.com"));
        assert!(!is_kindle_address("@kindle.com")); // empty local part
        assert!(!is_kindle_address("reader@kindle.com.evil.com"));
    }

    #[test]
    fn epub_filename_predicate() {
        assert!(is_epub_filename("My Book.epub"));
        assert!(is_epub_filename("MY BOOK.EPUB"));
        assert!(!is_epub_filename("book.mobi"));
        assert!(!is_epub_filename("book.pdf"));
    }

    // ---- config validators -------------------------------------------------

    #[test]
    fn valid_config_passes() {
        assert!(validate_config(&valid_config()).is_ok());
        assert!(is_configured(&valid_config()));
    }

    #[test]
    fn rejects_non_kindle_destination() {
        let mut cfg = valid_config();
        cfg.to_address = "reader@gmail.com".into();
        assert_eq!(
            validate_config(&cfg),
            Err(KindleConfigError::NotKindleAddress)
        );
        assert!(!is_configured(&cfg));
    }

    #[test]
    fn rejects_missing_fields_and_bad_port() {
        let cases: Vec<(fn(&mut KindleConfig), KindleConfigError)> = vec![
            (|c| c.smtp_host = "".into(), KindleConfigError::MissingHost),
            (|c| c.smtp_port = 0, KindleConfigError::InvalidPort),
            (
                |c| c.smtp_username = "".into(),
                KindleConfigError::MissingUsername,
            ),
            (
                |c| c.smtp_password = "".into(),
                KindleConfigError::MissingPassword,
            ),
            (|c| c.from_address = "".into(), KindleConfigError::MissingFrom),
        ];
        for (mutate, expected) in cases {
            let mut cfg = valid_config();
            mutate(&mut cfg);
            assert_eq!(validate_config(&cfg), Err(expected));
        }
    }

    // ---- pure MIME builder -------------------------------------------------

    #[test]
    fn builds_multipart_epub_attachment() {
        let msg = build_kindle_message(
            "user@example.com",
            "reader@kindle.com",
            "My Book",
            "My Book.epub",
            b"PK\x03\x04 epub bytes",
        )
        .expect("message builds");
        let raw = String::from_utf8_lossy(&msg.formatted()).to_string();

        assert!(raw.contains("From: user@example.com"));
        assert!(raw.contains("To: reader@kindle.com"));
        assert!(raw.contains("Subject: My Book"));
        assert!(raw.contains("multipart/mixed"));
        assert!(raw.contains("application/epub+zip"));
        assert!(raw.contains("Content-Disposition: attachment"));
        assert!(raw.contains("filename=\"My Book.epub\""));
    }

    #[test]
    fn build_rejects_bad_address() {
        let err = build_kindle_message(
            "not-an-address",
            "reader@kindle.com",
            "s",
            "b.epub",
            b"x",
        )
        .unwrap_err();
        assert!(err.contains("from address"));
    }

    // ---- orchestration with the fake sender --------------------------------

    #[test]
    fn happy_path_sends_once() {
        let sender = FakeSender::default();
        let outcome = block_on(send_epub_to_kindle(
            &valid_config(),
            "My Book",
            "My Book.epub",
            b"epub bytes",
            &sender,
        ));
        assert_eq!(outcome, SendOutcome::Sent);
        assert_eq!(*sender.calls.lock().unwrap(), 1);
        let sent = sender.last.lock().unwrap().clone().unwrap();
        assert!(String::from_utf8_lossy(&sent).contains("application/epub+zip"));
    }

    #[test]
    fn not_configured_does_not_send() {
        let mut cfg = valid_config();
        cfg.to_address = "reader@gmail.com".into(); // invalid → not configured
        let sender = FakeSender::default();
        let outcome = block_on(send_epub_to_kindle(
            &cfg,
            "My Book",
            "My Book.epub",
            b"epub bytes",
            &sender,
        ));
        assert_eq!(outcome, SendOutcome::NotConfigured);
        assert_eq!(*sender.calls.lock().unwrap(), 0);
    }

    #[test]
    fn non_epub_does_not_send() {
        let sender = FakeSender::default();
        let outcome = block_on(send_epub_to_kindle(
            &valid_config(),
            "My Book",
            "My Book.mobi",
            b"bytes",
            &sender,
        ));
        assert_eq!(outcome, SendOutcome::NotAnEpub);
        assert_eq!(*sender.calls.lock().unwrap(), 0);
    }

    #[test]
    fn oversized_does_not_send() {
        let sender = FakeSender::default();
        let big = vec![0u8; MAX_ATTACHMENT_BYTES + 1];
        let outcome = block_on(send_epub_to_kindle(
            &valid_config(),
            "My Book",
            "My Book.epub",
            &big,
            &sender,
        ));
        assert_eq!(
            outcome,
            SendOutcome::TooLarge {
                size: MAX_ATTACHMENT_BYTES + 1,
                limit: MAX_ATTACHMENT_BYTES,
            }
        );
        assert_eq!(*sender.calls.lock().unwrap(), 0);
    }

    #[test]
    fn transport_error_becomes_send_failed() {
        let sender = FakeSender {
            fail_with: Some("connection refused".into()),
            ..Default::default()
        };
        let outcome = block_on(send_epub_to_kindle(
            &valid_config(),
            "My Book",
            "My Book.epub",
            b"epub bytes",
            &sender,
        ));
        assert_eq!(
            outcome,
            SendOutcome::SendFailed {
                reason: "connection refused".into()
            }
        );
        assert_eq!(*sender.calls.lock().unwrap(), 1);
    }
}
