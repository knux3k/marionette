//! HTTP as plain data. Network plug-ins build [`HttpRequest`] values and parse
//! [`HttpResponse`] values; a [`Transport`] moves the bytes. Tests use a scripted transport, so
//! every plug-in is exercised with no network. A default build has no real transport
//! ([`NoNetwork`]); the `net` cargo feature adds [`Network`] (decisions 0019 and 0045).
//! [`default_transport`] picks the one this build has.

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

/// Largest answer body read into memory (a generated GLB can be tens of megabytes).
pub const MAX_BODY_BYTES: u64 = 256 * 1024 * 1024;

/// The real network transport (cargo feature `net`): HTTPS through `ureq` with rustls and the
/// web PKI roots. Non-2xx answers are returned as data, not errors, so plug-ins can report
/// the service's message. Follows redirects (download links often redirect).
#[cfg(feature = "net")]
pub struct Network {
    agent: ureq::Agent,
}

#[cfg(feature = "net")]
impl Network {
    pub fn new(timeout: std::time::Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(timeout))
            .user_agent("marionette-engine")
            .build();
        Self { agent: config.into() }
    }
}

#[cfg(feature = "net")]
impl Transport for Network {
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, String> {
        let mut builder = ureq::http::Request::builder()
            .method(match request.method {
                Method::Get => "GET",
                Method::Post => "POST",
            })
            .uri(&request.url);
        for h in &request.headers {
            builder = builder.header(&h.name, &h.value);
        }
        let req = builder
            .body(request.body.clone().unwrap_or_default())
            .map_err(|e| format!("bad request: {e}"))?;
        let mut resp = self.agent.run(req).map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let body = resp
            .body_mut()
            .with_config()
            .limit(MAX_BODY_BYTES)
            .read_to_vec()
            .map_err(|e| e.to_string())?;
        Ok(HttpResponse { status, body })
    }
}

/// The transport this build has: [`Network`] with the `net` feature, else [`NoNetwork`].
pub fn default_transport() -> Box<dyn Transport> {
    #[cfg(feature = "net")]
    {
        Box::new(Network::new(std::time::Duration::from_secs(300)))
    }
    #[cfg(not(feature = "net"))]
    {
        Box::new(NoNetwork)
    }
}
