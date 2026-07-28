use std::time::Duration;

use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor, message::Mailbox,
    transport::smtp::Error as SmtpError,
};
use thiserror::Error;
use tokio::time::sleep;

const SMTP_MAX_ATTEMPTS: u8 = 3;

#[derive(Clone)]
pub struct Mailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    frontend_url: String,
}

#[derive(Debug, Error)]
pub enum MailError {
    #[error("email address is invalid")]
    Address(#[from] lettre::address::AddressError),
    #[error("email message could not be built")]
    Message(#[from] lettre::error::Error),
    #[error("SMTP operation failed")]
    Smtp(#[from] SmtpError),
}

impl Mailer {
    /// Creates a reusable SMTP email client
    ///
    /// # Errors
    ///
    /// Returns an error when the SMTP URL or sender address is invalid
    pub fn new(smtp_url: &str, from: &str, frontend_url: String) -> Result<Self, MailError> {
        let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(smtp_url)?.build();
        let from = from.parse::<Mailbox>()?;

        Ok(Self {
            transport,
            from,
            frontend_url,
        })
    }

    /// Sends a confirmation link to a registered user
    ///
    /// # Errors
    ///
    /// Returns an error when the recipient is invalid, the message cannot be
    /// built, or the SMTP server rejects the message
    pub async fn send_email_confirmation(
        &self,
        recipient: &str,
        token: &str,
    ) -> Result<(), MailError> {
        let confirmation_url = format!(
            "{}/confirm-email?token={token}",
            self.frontend_url.trim_end_matches("/")
        );

        let message = Message::builder()
            .from(self.from.clone())
            .to(recipient.parse::<Mailbox>()?)
            .subject("Confirm your email")
            .body(format!(
                "Confirm you email by opening this link:\n\n
                {confirmation_url}\n\n
                This link expires in 30 minutes."
            ))?;

        self.send_message(message).await
    }

    /// Sends a password reset link to a registered user
    ///
    /// # Errors
    ///
    /// Returns an error when the recipient is invalid, the message cannot be
    /// built, or the SMTP server rejects the message
    pub async fn send_password_reset(&self, recipient: &str, token: &str) -> Result<(), MailError> {
        let reset_url = format!(
            "{}/reset-password?token={token}",
            self.frontend_url.trim_end_matches("/")
        );

        let message = Message::builder()
            .from(self.from.clone())
            .to(recipient.parse::<Mailbox>()?)
            .subject("Reset your password")
            .body(format!(
                "Reset your password by opening this link:\n\n
                {reset_url}\n\n
                This link expires in 30 minutes.\n\n
                Ignore this email if you did not request it."
            ))?;

        self.send_message(message).await
    }

    async fn send_message(&self, message: Message) -> Result<(), MailError> {
        let mut attempt = 1_u8;

        loop {
            match self.transport.send(message.clone()).await {
                Ok(_) => return Ok(()),

                Err(error) if attempt >= SMTP_MAX_ATTEMPTS => return Err(error.into()),

                Err(error) => {
                    tracing::warn!(
                        attempt,
                        max_attempts = SMTP_MAX_ATTEMPTS,
                        error = ?error,
                        "SMTP send failed; retrying..."
                    );

                    sleep(Duration::from_millis(250 * u64::from(attempt))).await;

                    attempt += 1;
                }
            }
        }
    }
}
