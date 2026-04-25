mod client;
mod proxy;
mod request;
mod stream;

pub use client::ChatCompletionsCompatClient;
pub use proxy::CHAT_WIRE_UPSTREAM_URL_HEADER;
pub use proxy::build_chat_completions_upstream_url;
pub use proxy::ensure_local_responses_proxy;
pub use request::ToolKinds;
pub use request::ToolOutputKind;
