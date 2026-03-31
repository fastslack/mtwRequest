use async_trait::async_trait;
use mtw_core::MtwError;
use crate::account::EmailAccount;
use crate::message::Communication;

#[async_trait]
pub trait MtwEmailService: Send + Sync {
    async fn send(&self, account: &EmailAccount, to: &[String], subject: &str, body: &str, html: Option<&str>) -> Result<String, MtwError>;
    async fn fetch_inbox(&self, account: &EmailAccount, limit: usize, offset: usize) -> Result<Vec<Communication>, MtwError>;
    async fn get_thread(&self, account: &EmailAccount, thread_id: &str) -> Result<Vec<Communication>, MtwError>;
}
