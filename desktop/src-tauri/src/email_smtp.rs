//! SMTP 邮件发送(lettre 实装)+ env 构造(Phase 4.3,对齐 reference `email_sender.py`)。
//!
//! `SMTP_HOST` 空 -> `InMemoryEmailSender`(回退,捕获不发);非空 -> `SmtpEmailSender`
//! (STARTTLS 或 plain,可选 credentials)。body=markdown plain text + 同一份 markdown 作
//! `.md` 附件(对齐 reference)。无 SMTP 服务器时无法单测,InMemory 在 engine-reports 测。

#![allow(missing_docs)]

use std::sync::Arc;

use async_trait::async_trait;
use lettre::message::header::{ContentDisposition, ContentType};
use lettre::message::{Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use engine_reports::{EmailError, EmailSender, InMemoryEmailSender};

/// SMTP 邮件发送器(lettre async tokio1)。
pub struct SmtpEmailSender {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl SmtpEmailSender {
    /// 从 env 构造:`SMTP_HOST`(必填)/ `SMTP_PORT`(默认 25)/ `SMTP_FROM` /
    /// `SMTP_USER`+`SMTP_PASSWORD`(可选)/ `SMTP_USE_TLS`(默认 false)。
    pub fn from_env() -> Result<Self, String> {
        let host = std::env::var("SMTP_HOST").unwrap_or_default();
        if host.is_empty() {
            return Err("SMTP_HOST 未设置".into());
        }
        let port: u16 = std::env::var("SMTP_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(25);
        let from_str =
            std::env::var("SMTP_FROM").unwrap_or_else(|_| "sre-platform@example.com".to_string());
        let from: Mailbox =
            from_str.parse().map_err(|e| format!("SMTP_FROM 解析失败: {e}"))?;
        let user = std::env::var("SMTP_USER").ok();
        let password = std::env::var("SMTP_PASSWORD").ok();
        let use_tls = std::env::var("SMTP_USE_TLS")
            .ok()
            .map(|s| s.to_lowercase() == "true")
            .unwrap_or(false);

        let mut builder = if use_tls {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host)
                .map_err(|e| format!("smtp starttls builder: {e}"))?
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&host)
        };
        builder = builder.port(port);
        if let (Some(u), Some(p)) = (user, password) {
            builder = builder.credentials(Credentials::new(u, p));
        }
        Ok(Self {
            transport: builder.build(),
            from,
        })
    }
}

#[async_trait]
impl EmailSender for SmtpEmailSender {
    async fn send(
        &self,
        recipients: Vec<String>,
        subject: &str,
        body: &str,
        attachment_filename: &str,
        attachment_content: &str,
    ) -> Result<(), EmailError> {
        let mut builder = Message::builder()
            .from(self.from.clone())
            .subject(subject.to_string());
        for r in &recipients {
            let mb: Mailbox = r
                .parse()
                .map_err(|e| EmailError::Smtp(format!("收件人 {r} 解析失败: {e}")))?;
            builder = builder.to(mb);
        }
        let email = builder
            .multipart(
                MultiPart::mixed()
                    .singlepart(SinglePart::plain(body.to_string()))
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_PLAIN)
                            .header(ContentDisposition::attachment(attachment_filename))
                            .body(attachment_content.to_string()),
                    ),
            )
            .map_err(|e| EmailError::Smtp(format!("build message: {e}")))?;
        self.transport
            .send(email)
            .await
            .map_err(|e| EmailError::Smtp(format!("smtp send: {e}")))?;
        Ok(())
    }
}

/// 从 env 构造 `EmailSender`:`SMTP_HOST` 空 -> `InMemory`(回退),非空 -> `Smtp`。
pub fn get_email_sender() -> Arc<dyn EmailSender> {
    match SmtpEmailSender::from_env() {
        Ok(smtp) => Arc::new(smtp),
        Err(e) => {
            tracing::info!("SMTP 未配置({e}),回退 InMemoryEmailSender(邮件捕获不发)");
            Arc::new(InMemoryEmailSender::new())
        }
    }
}
