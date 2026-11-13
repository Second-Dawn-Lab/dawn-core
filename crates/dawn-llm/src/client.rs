//! A small client for the OpenAI-style chat endpoint both backends offer (`llama-server` and Ollama), plus Ollama's
//! model list. Every request goes to an [`Endpoint`], which can only be this computer.

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Deserialize;

use crate::endpoint::Endpoint;
use crate::error::{LlmError, Result};
use crate::prompts::Prompt;

fn agent(read_timeout: Duration) -> ureq::Agent {
    // No proxy, no redirects: a local server has neither, and a redirect must never lead off this computer.
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(3)).timeout_read(read_timeout).redirects(0).try_proxy_from_env(false).build()
}

#[derive(Debug, Clone)]
pub struct Client {
    endpoint: Endpoint,
    /// The model name Ollama needs; empty for `llama-server`, which serves the one model it loaded.
    model: String,
}

impl Client {
    pub fn new(endpoint: Endpoint, model: impl Into<String>) -> Self {
        Self { endpoint, model: model.into() }
    }

    /// Ask for one answer, read as a stream so that `cancel` stops the generation at once (the connection is dropped and
    /// the server stops).
    pub fn complete(&self, prompt: &Prompt, cancel: &AtomicBool) -> Result<String> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{ "role": "user", "content": prompt.user }],
            "stream": true,
            "temperature": 0,
            "max_tokens": prompt.max_tokens,
            // Answers are plain text; reasoning models must not think first (llama-server / Ollama ignore what they do not know).
            "chat_template_kwargs": { "enable_thinking": false },
            "reasoning_effort": "none",
        });
        let response = agent(Duration::from_secs(120))
            .post(&self.endpoint.url("/v1/chat/completions"))
            .set("Content-Type", "application/json")
            .send_json(body)
            .map_err(|e| LlmError::Server(e.to_string()))?;
        let mut answer = String::new();
        for line in BufReader::new(response.into_reader()).lines() {
            if cancel.load(Ordering::Relaxed) {
                return Err(LlmError::Cancelled);
            }
            let line = line.map_err(|e| LlmError::Server(e.to_string()))?;
            let Some(data) = line.strip_prefix("data:") else { continue };
            let data = data.trim();
            if data == "[DONE]" {
                break;
            }
            if let Some(delta) = parse_delta(data) {
                answer.push_str(&delta);
            }
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(LlmError::Cancelled);
        }
        Ok(answer)
    }

    /// `llama-server` answers `/health` with 200 once the model is loaded (503 while it loads).
    pub fn is_ready(&self) -> bool {
        agent(Duration::from_secs(2)).get(&self.endpoint.url("/health")).call().is_ok()
    }
}

/// The text of one streamed chunk (`choices[0].delta.content`).
fn parse_delta(data: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(data).ok()?;
    v.pointer("/choices/0/delta/content")?.as_str().map(str::to_string)
}

#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
pub struct OllamaModel {
    pub name: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Deserialize)]
struct Tags {
    #[serde(default)]
    models: Vec<OllamaModel>,
}

/// The models a local Ollama has installed, or an error when none is running there. A short timeout: this runs when
/// the settings page opens, and nothing is installed or downloaded by it.
pub fn ollama_models(endpoint: &Endpoint) -> Result<Vec<OllamaModel>> {
    let response = agent(Duration::from_secs(3)).get(&endpoint.url("/api/tags")).call().map_err(|e| LlmError::Server(e.to_string()))?;
    let tags: Tags = response.into_json().map_err(|e| LlmError::Server(e.to_string()))?;
    Ok(tags.models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn stream_chunks_are_read() {
        assert_eq!(parse_delta(r#"{"choices":[{"delta":{"content":"Hal"}}]}"#).as_deref(), Some("Hal"));
        assert_eq!(parse_delta(r#"{"choices":[{"delta":{}}]}"#), None);
        assert_eq!(parse_delta("garbage"), None);
    }

    /// Read one whole HTTP request (headers and body) so the reply is not sent while the client is still writing.
    fn read_request(stream: &mut std::net::TcpStream) {
        let mut data = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = stream.read(&mut buf).unwrap_or(0);
            if n == 0 {
                return;
            }
            data.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&data).to_string();
            if let Some(head_end) = text.find("\r\n\r\n") {
                let length = text[..head_end]
                    .lines()
                    .find_map(|l| l.to_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()))
                    .unwrap_or(0);
                if data.len() >= head_end + 4 + length {
                    return;
                }
            }
        }
    }

    /// A local server that answers every request with `body`.
    fn serve(body: &'static str, content_type: &'static str) -> Endpoint {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                read_request(&mut stream);
                let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        Endpoint::local(port)
    }

    #[test]
    fn a_streamed_answer_is_collected_and_cancel_stops_it() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"Die \"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"Eule.\"}}]}\n\ndata: [DONE]\n\n";
        let ep = serve(sse, "text/event-stream");
        let client = Client::new(ep, "m");
        let prompt = Prompt { user: "x".into(), max_tokens: 10 };
        assert_eq!(client.complete(&prompt, &AtomicBool::new(false)).unwrap(), "Die Eule.");
        assert_eq!(client.complete(&prompt, &AtomicBool::new(true)), Err(LlmError::Cancelled));
    }

    #[test]
    fn ollama_models_are_listed() {
        let ep = serve(r#"{"models":[{"name":"qwen3:4b","size":2500000000},{"name":"llama3.2:3b","size":2000000000}]}"#, "application/json");
        let models = ollama_models(&ep).unwrap();
        assert_eq!(models.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["qwen3:4b", "llama3.2:3b"]);
    }

    #[test]
    fn no_server_is_an_error_not_a_hang() {
        // Port 1 on this computer: nothing listens.
        assert!(matches!(ollama_models(&Endpoint::local(1)), Err(LlmError::Server(_))));
    }
}
