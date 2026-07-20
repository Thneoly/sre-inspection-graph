//! 邮件发送抽象 + InMemory 实现(PRD-003 Phase 4.3,对齐 reference `email_sender.py`)。
//!
//! `EmailSender` async trait + `InMemoryEmailSender`(纯,捕获已发邮件供调试;默认回退)。
//! `SmtpEmailSender`(lettre 实装)在 desktop 层(I/O 依赖不进 engine-reports)。
//! 无时钟依赖(不调 `Utc::now()`,sent_at 由调用方管;调试视图按插入序)。

#![allow(missing_docs)]

use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::Mutex;

/// 已发送邮件快照(InMemory 模式捕获;调试视图用)。
#[derive(Debug, Clone, Serialize)]
pub struct SentEmail {
    pub recipients: Vec<String>,
    pub subject: String,
    pub body: String,
    pub attachment_filename: String,
    pub attachment_content: String,
}

/// 邮件发送错误。
#[derive(Debug, thiserror::Error)]
pub enum EmailError {
    /// SMTP 传输错误(lettre 实装)。
    #[error("smtp error: {0}")]
    Smtp(String),
}

/// 邮件发送抽象(对齐 reference `EmailSender` ABC)。
///
/// body 为 Markdown plain text;attachment 为同一份 Markdown 存成 `.md`(对齐 reference:
/// body 和 attachment 都是 `task.markdown`)。
#[async_trait]
pub trait EmailSender: Send + Sync {
    /// 发送邮件。`body` = Markdown plain text;`attachment_filename`/`attachment_content`
    /// = 同一份 Markdown 作 `.md` 附件。
    async fn send(
        &self,
        recipients: Vec<String>,
        subject: &str,
        body: &str,
        attachment_filename: &str,
        attachment_content: &str,
    ) -> Result<(), EmailError>;

    /// 已发送邮件列表(InMemory 模式返捕获;Smtp 模式返空)。
    async fn list_sent(&self) -> Vec<SentEmail> {
        Vec::new()
    }
}

/// 内存邮件发送器(默认 + 测试;捕获已发邮件供调试,对齐 reference `InMemoryEmailSender`)。
#[derive(Debug, Default)]
pub struct InMemoryEmailSender {
    sent: Mutex<Vec<SentEmail>>,
}

impl InMemoryEmailSender {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl EmailSender for InMemoryEmailSender {
    async fn send(
        &self,
        recipients: Vec<String>,
        subject: &str,
        body: &str,
        attachment_filename: &str,
        attachment_content: &str,
    ) -> Result<(), EmailError> {
        self.sent.lock().await.push(SentEmail {
            recipients,
            subject: subject.to_string(),
            body: body.to_string(),
            attachment_filename: attachment_filename.to_string(),
            attachment_content: attachment_content.to_string(),
        });
        Ok(())
    }

    async fn list_sent(&self) -> Vec<SentEmail> {
        self.sent.lock().await.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_captures_and_lists() {
        let sender = InMemoryEmailSender::new();
        assert!(sender.list_sent().await.is_empty());
        sender
            .send(
                vec!["a@b.c".into(), "d@e.f".into()],
                "subject",
                "# body",
                "rpt-1.md",
                "# body",
            )
            .await
            .unwrap();
        sender
            .send(vec!["x@y.z".into()], "s2", "body2", "rpt-2.md", "body2")
            .await
            .unwrap();
        let sent = sender.list_sent().await;
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].recipients, vec!["a@b.c".to_string(), "d@e.f".to_string()]);
        assert_eq!(sent[0].subject, "subject");
        assert_eq!(sent[0].body, "# body");
        assert_eq!(sent[0].attachment_filename, "rpt-1.md");
        assert_eq!(sent[0].attachment_content, "# body");
        assert_eq!(sent[1].recipients, vec!["x@y.z".to_string()]);
    }

    #[tokio::test]
    async fn default_list_sent_is_empty_for_override() {
        // 默认 trait impl 返空(SmtpEmailSender 在 desktop 测)
        struct Dummy;
        #[async_trait]
        impl EmailSender for Dummy {
            async fn send(
                &self,
                _r: Vec<String>,
                _s: &str,
                _b: &str,
                _af: &str,
                _ac: &str,
            ) -> Result<(), EmailError> {
                Ok(())
            }
        }
        assert!(Dummy.list_sent().await.is_empty());
    }
}
