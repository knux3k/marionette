//! HTTP as plain data. Network plug-ins build [`HttpRequest`] values and parse
//! [`HttpResponse`] values; a [`Transport`] moves the bytes. Tests use a scripted transport, so
//! every plug-in is exercised with no network. This build has no real transport
//! ([`NoNetwork`]); see decision 0019.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub value: String,
    /// Holds a key: never printed (see the `Debug` impl).
    pub secret: bool,
}

impl fmt::Debug for Header {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = if self.secret {
            "<redacted>"
        } else {
            self.value.as_str()
        };
        write!(f, "{}: {}", self.name, value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<Header>,
    pub body: Option<Vec<u8>>,
}

impl HttpRequest {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: Method::Get,
            url: url.into(),
            headers: Vec::new(),
            body: None,
        }
    }

    pub fn post_json(url: impl Into<String>, body: &serde_json::Value) -> Self {
        Self {
            method: Method::Post,
            url: url.into(),
            headers: vec![Header {
                name: "content-type".into(),
                value: "application/json".into(),
                secret: false,
            }],
            body: Some(serde_json::to_vec(body).expect("JSON values serialize")),
        }
    }

    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push(Header {
            name: name.into(),
            value: value.into(),
            secret: false,
        });
        self
    }

    pub fn secret_header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push(Header {
            name: name.into(),
            value: value.into(),
            secret: true,
        });
        self
    }

    /// The value of a header (case-insensitive name).
    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    }

    pub fn body_json(&self) -> Option<serde_json::Value> {
        self.body.as_ref().and_then(|b| serde_json::from_slice(b).ok())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn json(status: u16, body: &serde_json::Value) -> Self {
        Self {
            status,
            body: serde_json::to_vec(body).expect("JSON values serialize"),
        }
    }
}

/// Moves one request and its answer.
pub trait Transport {
    /// False when this transport cannot reach the network at all.
    fn available(&self) -> bool {
        true
    }
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, String>;
}

/// The transport of this build: no network.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoNetwork;

impl Transport for NoNetwork {
    fn available(&self) -> bool {
        false
    }
    fn send(&self, _: &HttpRequest) -> Result<HttpResponse, String> {
        Err("this build has no network transport".into())
    }
}
