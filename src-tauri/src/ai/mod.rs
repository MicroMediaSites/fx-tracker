mod client;
pub mod context;
mod prompts;
mod sanitize;
pub mod strategy_builder;
pub mod strategy_recovery;
pub mod streaming;

pub use client::{ClaudeClient, ChatMessage, AiTier, HelpResponse};
pub use context::ChatContext;
pub use streaming::{
    StreamingClaudeClient, ChatTokenEvent, ChatCompleteEvent, ChatErrorEvent,
    StreamResult, ToolUseRequest, ApiMessage, ApiContent, ContentBlock, ApiTool,
};
pub use prompts::{
    BACKTEST_ANALYSIS_PROMPT, ANALYSIS_WHAT_WENT_WRONG,
    ANALYSIS_EXPLAIN_METRICS, ANALYSIS_CUSTOM, ANALYSIS_PERIOD_COMPARISON,
};
pub use sanitize::{
    sanitize_user_input, sanitize_chat_message, wrap_user_input,
    log_suspicious_input, SanitizedInput,
    // Analysis question classifier (Haiku pre-check)
    ANALYSIS_CLASSIFIER_PROMPT, AnalysisClassificationResult, parse_analysis_classification,
};
pub use strategy_builder::{
    StrategyAssistRequest, StrategyAssistResponse, ConversationMessage,
    STRATEGY_BUILDER_SYSTEM_PROMPT, parse_strategy_response,
    // Two-stage classification
    ClassificationResult, CLASSIFIER_PROMPT, parse_classification_response,
};
pub use strategy_recovery::RecoveryResult;
