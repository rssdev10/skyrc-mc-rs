#[derive(Debug, Clone)]
pub enum Message {
    // Re-export app messages for UI components
    AppMessage(crate::app::AppMessage),
}