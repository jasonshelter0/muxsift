use std::time::Duration;

use reqwest::{
    Client,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue},
};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct AiOptions {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub system_prompt: String,
}

#[derive(Clone)]
pub struct AiClient {
    client: Client,
    options: AiOptions,
}

impl AiClient {
    pub fn new(options: AiOptions) -> Result<Self> {
        if options.base_url.trim().is_empty()
            || options.model.trim().is_empty()
            || options.api_key.trim().is_empty()
        {
            return Err(Error::InvalidInput(
                "AI configuration requires base_url, model, and an API key".into(),
            ));
        }
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", options.api_key))
                .map_err(|_| Error::InvalidInput("invalid AI API key".into()))?,
        );
        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|error| {
                Error::InvalidInput(format!("could not initialize AI client: {error}"))
            })?;
        Ok(Self { client, options })
    }

    pub async fn test_connection(&self) -> Result<()> {
        let reply = self
            .complete("Reply with exactly OK.", "Connection test")
            .await?;
        if reply.trim().is_empty() {
            return Err(Error::InvalidInput(
                "AI connection test returned an empty response".into(),
            ));
        }
        Ok(())
    }

    pub async fn srt_to_markdown(&self, srt: &str) -> Result<String> {
        self.complete(&self.options.system_prompt, srt).await
    }

    async fn complete(&self, system: &str, user: &str) -> Result<String> {
        let url = format!(
            "{}/chat/completions",
            self.options.base_url.trim_end_matches('/')
        );
        let body = ChatRequest {
            model: &self.options.model,
            messages: [
                Message {
                    role: "system",
                    content: system,
                },
                Message {
                    role: "user",
                    content: user,
                },
            ],
            temperature: 0.2,
        };
        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|error| Error::InvalidInput(format!("AI request failed: {error}")))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| Error::InvalidInput(format!("AI response read failed: {error}")))?;
        if !status.is_success() {
            return Err(Error::InvalidInput(format!(
                "AI request failed: HTTP {status}: {}",
                text.chars().take(400).collect::<String>()
            )));
        }
        let response: ChatResponse = serde_json::from_str(&text)
            .map_err(|error| Error::InvalidInput(format!("invalid AI response: {error}")))?;
        response
            .choices
            .into_iter()
            .next()
            .map(|choice| choice.message.content)
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| Error::InvalidInput("AI response contained no message content".into()))
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: [Message<'a>; 2],
    temperature: f32,
}
#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}
#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}
#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}
#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}
