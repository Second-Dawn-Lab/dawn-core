//! Where the local model listens. Only this computer is allowed: `127.0.0.1`, `localhost` or `[::1]`. A remote
//! address is refused before any connection is made, so no text can leave the machine through this crate.

use crate::error::{LlmError, Result};

pub const OLLAMA_DEFAULT: &str = "127.0.0.1:11434";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// `127.0.0.1`, `localhost` or `[::1]`.
    host: String,
    port: u16,
}

impl Endpoint {
    /// `127.0.0.1:11434`, `localhost:11434` or `http://localhost:11434` (a path is ignored).
    pub fn parse(text: &str) -> Result<Self> {
        let raw = text.trim();
        let rest = raw.strip_prefix("http://").unwrap_or(raw);
        if rest.starts_with("https://") || raw.starts_with("https://") {
            return Err(LlmError::NotLocal(text.to_string()));
        }
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        // A user name in front of the host would hide the real host ("localhost@evil.example").
        if authority.contains('@') {
            return Err(LlmError::NotLocal(text.to_string()));
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) if !h.ends_with(':') || h.starts_with('[') => (h, p.parse::<u16>().map_err(|_| LlmError::NotLocal(text.to_string()))?),
            _ => (authority, 80),
        };
        let host = host.to_lowercase();
        if !matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]") || port == 0 {
            return Err(LlmError::NotLocal(text.to_string()));
        }
        Ok(Self { host, port })
    }

    pub fn local(port: u16) -> Self {
        Self { host: "127.0.0.1".into(), port }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}:{}{}", self.host, self.port, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_addresses_are_accepted() {
        assert_eq!(Endpoint::parse("127.0.0.1:11434").unwrap().url("/api/tags"), "http://127.0.0.1:11434/api/tags");
        assert_eq!(Endpoint::parse("localhost:11434").unwrap().port(), 11434);
        assert_eq!(Endpoint::parse("http://localhost:8080/v1").unwrap().url("/x"), "http://localhost:8080/x");
        assert_eq!(Endpoint::parse("[::1]:11434").unwrap().url(""), "http://[::1]:11434");
        assert_eq!(Endpoint::parse(" LOCALHOST:1 ").unwrap().port(), 1);
    }

    #[test]
    fn everything_else_is_refused_before_any_connection() {
        for bad in [
            "192.168.1.5:11434",
            "10.0.0.2:11434",
            "example.com:11434",
            "ollama.local:11434",
            "https://localhost:11434",
            "localhost.evil.example:11434",
            "127.0.0.1.evil.example:11434",
            "localhost@evil.example:11434",
            "0.0.0.0:11434",
            "127.0.0.2:11434",
            "",
            "localhost:notaport",
            "localhost:0",
        ] {
            assert!(matches!(Endpoint::parse(bad), Err(LlmError::NotLocal(_))), "{bad:?} must be refused");
        }
    }
}
