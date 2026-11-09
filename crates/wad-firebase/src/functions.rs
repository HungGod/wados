//! Calling a callable Cloud Function over HTTPS: POST {"data": ...}, answered
//! {"result": ...} or {"error": {"status", "message"}}.

use serde_json::{Value, json};

use crate::Error;

#[derive(Debug, Clone)]
pub struct Functions {
    http: reqwest::Client,
    /// https://<region>-<project>.cloudfunctions.net, or the emulator's
    /// http://127.0.0.1:5001/<project>/<region>.
    base: String,
}

impl Functions {
    pub fn new(http: reqwest::Client, base: &str) -> Self {
        Self { http, base: base.trim_end_matches('/').into() }
    }

    pub fn for_project(http: reqwest::Client, project: &str, region: &str) -> Self {
        Self::new(http, &format!("https://{region}-{project}.cloudfunctions.net"))
    }

    pub fn emulator(http: reqwest::Client, host: &str, project: &str, region: &str) -> Self {
        Self::new(http, &format!("http://{host}/{project}/{region}"))
    }

    pub async fn call(&self, name: &str, data: Value) -> Result<Value, Error> {
        let res = self
            .http
            .post(format!("{}/{name}", self.base))
            .json(&json!({ "data": data }))
            .send()
            .await
            .map_err(Error::Network)?;
        let status = res.status().as_u16();
        let body: Value = res.json().await.unwrap_or(Value::Null);
        if status == 200
            && let Some(r) = body.get("result")
        {
            return Ok(r.clone());
        }
        let err = body.get("error").cloned().unwrap_or(Value::Null);
        Err(Error::Callable {
            status: err.get("status").and_then(Value::as_str).unwrap_or("UNKNOWN").into(),
            message: err
                .get("message")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or_else(|| format!("HTTP {status}")),
        })
    }
}
