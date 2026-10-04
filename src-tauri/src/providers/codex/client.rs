use std::time::Duration;

use reqwest::{blocking::Client, StatusCode};
use serde_json::Value;

use super::CodexError;

const RESET_CREDITS_URL: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";
const CONSUME_RESET_CREDIT_URL: &str =
    "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits/consume";

#[derive(Debug, Clone)]
pub struct UsageResponse {
    pub status: StatusCode,
    pub body: Value,
}

pub struct CodexClient {
    client: Client,
    reset_credits_url: String,
    consume_reset_credit_url: String,
}

impl CodexClient {
    pub fn new() -> Result<Self, CodexError> {
        Self::with_endpoints(
            RESET_CREDITS_URL,
            CONSUME_RESET_CREDIT_URL,
            Duration::from_secs(15),
        )
    }

    fn with_endpoints(
        reset_credits_url: &str,
        consume_reset_credit_url: &str,
        timeout: Duration,
    ) -> Result<Self, CodexError> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(timeout)
            .user_agent(concat!("OpenQuota/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| CodexError::ConnectionFailed)?;
        Ok(Self {
            client,
            reset_credits_url: reset_credits_url.to_owned(),
            consume_reset_credit_url: consume_reset_credit_url.to_owned(),
        })
    }

    pub fn fetch_reset_credits(
        &self,
        access_token: &str,
        account_id: Option<&str>,
    ) -> Result<UsageResponse, CodexError> {
        let started = std::time::Instant::now();
        let mut request = self
            .client
            .get(&self.reset_credits_url)
            .bearer_auth(access_token)
            .header("Accept", "application/json")
            .header("OpenAI-Beta", "codex-1")
            .header("originator", "Codex Desktop");
        if let Some(account_id) = account_id.filter(|value| !value.is_empty()) {
            request = request.header("ChatGPT-Account-Id", account_id);
        }
        let response = request.send().map_err(|_| {
            crate::app_warn!("http", "codex reset-credit request failed (transport)");
            CodexError::ConnectionFailed
        })?;
        let status = response.status();
        crate::app_debug!(
            "http",
            "codex reset-credit HTTP {} ({}ms)",
            status.as_u16(),
            started.elapsed().as_millis()
        );
        let text = response.text().map_err(|_| CodexError::InvalidResponse)?;
        let body = serde_json::from_str(&text).unwrap_or(Value::Null);
        if status.is_success() && body.is_null() {
            return Err(CodexError::InvalidResponse);
        }
        Ok(UsageResponse { status, body })
    }

    pub fn consume_reset_credit(
        &self,
        access_token: &str,
        account_id: Option<&str>,
        credit_id: &str,
        redeem_request_id: &str,
    ) -> Result<UsageResponse, CodexError> {
        let started = std::time::Instant::now();
        let mut request = self
            .client
            .post(&self.consume_reset_credit_url)
            .bearer_auth(access_token)
            .header("Accept", "application/json")
            .header("OpenAI-Beta", "codex-1")
            .header("originator", "Codex Desktop")
            .json(&serde_json::json!({
                "credit_id": credit_id,
                "redeem_request_id": redeem_request_id,
            }));
        if let Some(account_id) = account_id.filter(|value| !value.is_empty()) {
            request = request.header("ChatGPT-Account-Id", account_id);
        }
        let response = request.send().map_err(|_| {
            crate::app_warn!("http", "codex reset-credit consume failed (transport)");
            CodexError::ConnectionFailed
        })?;
        let status = response.status();
        crate::app_debug!(
            "http",
            "codex reset-credit consume HTTP {} ({}ms)",
            status.as_u16(),
            started.elapsed().as_millis()
        );
        let text = response.text().map_err(|_| CodexError::InvalidResponse)?;
        let body = serde_json::from_str(&text).unwrap_or(Value::Null);
        Ok(UsageResponse { status, body })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::Duration,
    };

    use super::CodexClient;

    fn client(base: &str) -> CodexClient {
        CodexClient::with_endpoints(
            &format!("{base}/reset-credits"),
            &format!("{base}/reset-credits/consume"),
            Duration::from_secs(1),
        )
        .unwrap()
    }

    fn capture_once(body: &str) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let body = body.to_owned();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0_u8; 1024];
                let count = stream.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..count]);
                let text = String::from_utf8_lossy(&request);
                let Some(header_end) = text.find("\r\n\r\n") else {
                    continue;
                };
                let content_length = text[..header_end]
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|value| value.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            sender
                .send(String::from_utf8_lossy(&request).into_owned())
                .unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        (format!("http://{address}"), receiver)
    }

    #[test]
    fn reset_credit_consume_targets_one_credit_with_an_idempotency_key() {
        let (base, request) = capture_once(r#"{"code":"reset"}"#);
        let response = client(&base)
            .consume_reset_credit("secret-token", Some("account-id"), "credit-1", "redeem-1")
            .unwrap();
        assert_eq!(response.body["code"], "reset");

        let request = request.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(request.starts_with("POST /reset-credits/consume HTTP/1.1"));
        assert!(request.contains("\"credit_id\":\"credit-1\""));
        assert!(request.contains("\"redeem_request_id\":\"redeem-1\""));
        assert!(request
            .to_ascii_lowercase()
            .contains("openai-beta: codex-1"));
        assert!(request
            .to_ascii_lowercase()
            .contains("chatgpt-account-id: account-id"));
    }
}
