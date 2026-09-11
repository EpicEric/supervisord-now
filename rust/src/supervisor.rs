use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use http_body_util::{BodyExt, Full};
use hyper::Method;
use hyper::header::CONTENT_TYPE;
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioExecutor, TokioIo};
use quick_xml::XmlVersion;
use serde_json::Value;
use std::task::{Context, Poll};
use std::{future::Future, pin::Pin};

use crate::error::{ApiError, ApiResult};

#[derive(Clone)]
struct UnixConnector {
    socket_path: String,
}

impl tower::Service<hyper::Uri> for UnixConnector {
    type Response = TokioIo<tokio::net::UnixStream>;
    type Error = std::io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: hyper::Uri) -> Self::Future {
        let path = self.socket_path.clone();
        Box::pin(async move {
            let stream = tokio::net::UnixStream::connect(path).await?;
            Ok(TokioIo::new(stream))
        })
    }
}

#[derive(Debug, Clone)]
pub enum Param {
    Str(String),
    Int(i64),
    Bool(bool),
}

impl Param {
    fn to_xml(&self) -> String {
        match self {
            Param::Str(s) => format!("<value><string>{}</string></value>", xml_escape(s)),
            Param::Int(n) => format!("<value><int>{n}</int></value>"),
            Param::Bool(b) => format!(
                "<value><boolean>{}</boolean></value>",
                if *b { 1 } else { 0 }
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub group: String,
    pub state: i64,
    pub statename: String,
    pub description: String,
    pub exitstatus: i64,
    pub spawnerr: String,
}

impl ProcessInfo {
    fn from_value(v: Value) -> Option<Self> {
        Some(Self {
            group: string_field(&v, "group")?,
            state: number_field(&v, "state").unwrap_or_default(),
            statename: string_field(&v, "statename").unwrap_or_default(),
            description: string_field(&v, "description").unwrap_or_default(),
            exitstatus: number_field(&v, "exitstatus").unwrap_or_default(),
            spawnerr: string_field(&v, "spawnerr").unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct TailLog {
    pub data: String,
    pub offset: i64,
    pub overflow: bool,
}

#[derive(Clone)]
pub struct SupervisorClient {
    http: Client<UnixConnector, Full<bytes::Bytes>>,
}

impl SupervisorClient {
    pub fn new(socket_path: String) -> Self {
        let connector = UnixConnector {
            socket_path: socket_path.clone(),
        };
        let http = Client::builder(TokioExecutor::new()).build(connector);
        Self { http }
    }

    pub async fn call(&self, method: &str, params: &[Param]) -> Result<Value, ApiError> {
        let params_xml = if params.is_empty() {
            String::new()
        } else {
            let inner: String = params
                .iter()
                .map(|p| format!("<param>{}</param>", p.to_xml()))
                .collect();
            format!("<params>{inner}</params>")
        };
        let body = format!(
            "<?xml version=\"1.0\"?><methodCall><methodName>{method}</methodName>{params_xml}</methodCall>"
        );

        let request = hyper::Request::builder()
            .method(Method::POST)
            .uri("http://supervisord/RPC2")
            .header(CONTENT_TYPE, "text/xml")
            .body(Full::new(body.into_bytes().into()))
            .map_err(|e| ApiError::internal(format!("build rpc request: {e}")))?;

        let response = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            self.http.request(request),
        )
        .await
        .map_err(|_| ApiError::internal("supervisord rpc timed out"))?
        .map_err(|e| ApiError::internal(format!("supervisord rpc failed: {e}")))?;

        if !response.status().is_success() {
            return Err(ApiError::internal(format!(
                "supervisord returned HTTP {}",
                response.status()
            )));
        }

        let bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|e| ApiError::internal(format!("read rpc response: {e}")))?
            .to_bytes();
        parse_method_response(&String::from_utf8_lossy(&bytes))
    }

    async fn first_param(&self, method: &str, params: &[Param]) -> Result<Value, ApiError> {
        let values = self.call(method, params).await?;
        Ok(values
            .as_array()
            .and_then(|a| a.first().cloned())
            .unwrap_or(Value::Null))
    }

    pub async fn get_all_process_info(&self) -> Result<Vec<ProcessInfo>, ApiError> {
        let value = self
            .first_param("supervisor.getAllProcessInfo", &[])
            .await?;
        Ok(value
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(ProcessInfo::from_value)
            .collect())
    }

    pub async fn reload_config(&self) -> ApiResult<()> {
        self.first_param("supervisor.reloadConfig", &[])
            .await
            .map(|_| ())
    }

    pub async fn add_process_group(&self, name: &str) -> ApiResult<()> {
        self.first_param("supervisor.addProcessGroup", &[Param::Str(name.into())])
            .await
            .map(|_| ())
    }

    pub async fn remove_process_group(&self, name: &str) -> ApiResult<()> {
        self.first_param("supervisor.removeProcessGroup", &[Param::Str(name.into())])
            .await
            .map(|_| ())
    }

    pub async fn start_process_group(&self, name: &str, wait: bool) -> ApiResult<()> {
        self.first_param(
            "supervisor.startProcessGroup",
            &[Param::Str(name.into()), Param::Bool(wait)],
        )
        .await
        .map(|_| ())
    }

    pub async fn stop_process_group(&self, name: &str, wait: bool) -> ApiResult<()> {
        self.first_param(
            "supervisor.stopProcessGroup",
            &[Param::Str(name.into()), Param::Bool(wait)],
        )
        .await
        .map(|_| ())
    }

    pub async fn tail_process_stdout_log(
        &self,
        name: &str,
        offset: i64,
        length: i64,
    ) -> Result<TailLog, ApiError> {
        // The reply is spread over three xml-rpc params: (data, offset, overflow).
        let values = self
            .call(
                "supervisor.tailProcessStdoutLog",
                &[
                    Param::Str(name.into()),
                    Param::Int(offset),
                    Param::Int(length),
                ],
            )
            .await?;
        let params = values.as_array().cloned().unwrap_or_default();
        Ok(TailLog {
            data: params
                .first()
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            offset: params.get(1).and_then(Value::as_i64).unwrap_or_default(),
            overflow: params.get(2).and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

fn string_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn number_field(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(Value::as_i64)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn parse_method_response(xml: &str) -> Result<Value, ApiError> {
    let mut parser = ResponseParser::new(xml.as_bytes());
    parser.parse_response()
}

struct ResponseParser<'a> {
    reader: quick_xml::Reader<&'a [u8]>,
}

impl<'a> ResponseParser<'a> {
    fn new(xml: &'a [u8]) -> Self {
        // trim_text must stay off: it would eat whitespace adjacent to
        // entity references inside log payloads.
        let reader = quick_xml::Reader::from_reader(xml);
        Self { reader }
    }

    fn parse_response(&mut self) -> Result<Value, ApiError> {
        loop {
            let event = self.read_event()?;
            match event {
                quick_xml::events::Event::Start(e) => match e.name().as_ref() {
                    "params" => return self.parse_params(),
                    "fault" => {
                        let value = self.parse_fault_value()?;
                        let code = number_field(&value, "faultCode").unwrap_or(0);
                        let message = string_field(&value, "faultString").unwrap_or_default();
                        return Err(ApiError::internal(format!(
                            "supervisord fault {code}: {message}"
                        )));
                    }
                    _ => {}
                },
                quick_xml::events::Event::Eof => {
                    return Err(ApiError::internal("unexpected end of xml-rpc response"));
                }
                _ => {}
            }
        }
    }

    fn parse_params(&mut self) -> Result<Value, ApiError> {
        let mut items = Vec::new();
        loop {
            let event = self.read_event()?;
            match event {
                quick_xml::events::Event::Start(e) => {
                    if e.name().as_ref() == "param" {
                        items.push(self.parse_param()?);
                    }
                }
                quick_xml::events::Event::End(e) => {
                    if e.name().as_ref() == "params" {
                        return Ok(Value::Array(items));
                    }
                }
                quick_xml::events::Event::Eof => {
                    return Err(ApiError::internal("unexpected end of xml-rpc response"));
                }
                _ => {}
            }
        }
    }

    fn parse_param(&mut self) -> Result<Value, ApiError> {
        loop {
            let event = self.read_event()?;
            match event {
                quick_xml::events::Event::Start(e) => {
                    if e.name().as_ref() == "value" {
                        let value = self.parse_value()?;
                        self.expect_end_tag("param")?;
                        return Ok(value);
                    }
                }
                quick_xml::events::Event::Eof => {
                    return Err(ApiError::internal("unexpected end of xml-rpc response"));
                }
                _ => {}
            }
        }
    }

    // <fault><value><struct>...</struct></value></fault>: consume the
    // <value> wrapper, then parse the struct.
    fn parse_fault_value(&mut self) -> Result<Value, ApiError> {
        loop {
            let event = self.read_event()?;
            match event {
                quick_xml::events::Event::Start(e) => {
                    if e.name().as_ref() == "value" {
                        return self.parse_value();
                    }
                }
                quick_xml::events::Event::Eof => {
                    return Err(ApiError::internal("unexpected end of xml-rpc response"));
                }
                _ => {}
            }
        }
    }

    fn parse_value(&mut self) -> Result<Value, ApiError> {
        let mut shorthand = String::new();
        loop {
            let event = self.read_event()?;
            match event {
                quick_xml::events::Event::Text(t) => {
                    let decoded = t.xml_content(XmlVersion::Implicit1_0).into_owned();
                    if shorthand.is_empty() && decoded.trim().is_empty() {
                        continue;
                    }
                    shorthand.push_str(&decoded);
                }
                quick_xml::events::Event::CData(t) => {
                    shorthand.push_str(&t.xml_content(XmlVersion::Implicit1_0));
                }
                quick_xml::events::Event::GeneralRef(t) => {
                    if let Ok(Some(ch)) = t.resolve_char_ref() {
                        shorthand.push(ch);
                    } else if let Some(resolved) =
                        quick_xml::escape::resolve_predefined_entity(t.as_ref())
                    {
                        shorthand.push_str(resolved);
                    }
                }
                quick_xml::events::Event::Start(e) => match e.name().as_ref() {
                    "string" => {
                        let text = self.read_text_until("string")?;
                        self.expect_end_tag("value")?;
                        return Ok(Value::String(text));
                    }
                    "int" | "i4" => {
                        let text = self.read_text_until("int")?;
                        self.expect_end_tag("value")?;
                        let n = text.trim().parse::<i64>().map_err(|e| {
                            ApiError::internal(format!("bad xml-rpc int '{text}': {e}"))
                        })?;
                        return Ok(Value::Number(n.into()));
                    }
                    "boolean" => {
                        let text = self.read_text_until("boolean")?;
                        self.expect_end_tag("value")?;
                        return Ok(Value::Bool(text.trim() == "1"));
                    }
                    "double" => {
                        let text = self.read_text_until("double")?;
                        self.expect_end_tag("value")?;
                        return Ok(Value::String(text));
                    }
                    "base64" => {
                        let text = self.read_text_until("base64")?;
                        self.expect_end_tag("value")?;
                        let decoded = BASE64
                            .decode(text.trim().as_bytes())
                            .unwrap_or_else(|_| text.trim().as_bytes().to_vec());
                        return Ok(Value::String(String::from_utf8_lossy(&decoded).to_string()));
                    }
                    "array" => return self.parse_array_body(),
                    "struct" => return self.parse_struct_body(),
                    _ => {
                        self.skip_until("value")?;
                        return Ok(Value::Null);
                    }
                },
                quick_xml::events::Event::End(e) if e.name().as_ref() == "value" => {
                    return Ok(Value::String(shorthand));
                }
                quick_xml::events::Event::Eof => {
                    return Err(ApiError::internal("unexpected end of xml-rpc response"));
                }
                _ => {
                    self.expect_end_tag("value")?;
                    return Ok(Value::Null);
                }
            }
        }
    }

    fn parse_array_body(&mut self) -> Result<Value, ApiError> {
        let mut items = Vec::new();
        loop {
            let event = self.read_event()?;
            match event {
                quick_xml::events::Event::Start(e) => match e.name().as_ref() {
                    "data" => {}
                    "value" => items.push(self.parse_value()?),
                    _ => {}
                },
                quick_xml::events::Event::End(e) => {
                    if e.name().as_ref() == "array" {
                        self.expect_end_tag("value")?;
                        return Ok(Value::Array(items));
                    }
                }
                quick_xml::events::Event::Eof => {
                    return Err(ApiError::internal("unexpected end of xml-rpc response"));
                }
                _ => {}
            }
        }
    }

    fn parse_struct_body(&mut self) -> Result<Value, ApiError> {
        let mut map = serde_json::Map::new();
        let mut current_name: Option<String> = None;
        loop {
            let event = self.read_event()?;
            match event {
                quick_xml::events::Event::Start(e) => match e.name().as_ref() {
                    "member" => {}
                    "name" => {
                        let name = self.read_text_until("name")?;
                        current_name = Some(name);
                    }
                    "value" => {
                        let value = self.parse_value()?;
                        if let Some(name) = current_name.take() {
                            map.insert(name, value);
                        }
                    }
                    _ => {}
                },
                quick_xml::events::Event::End(e) => {
                    if e.name().as_ref() == "struct" {
                        self.expect_end_tag("value")?;
                        return Ok(Value::Object(map));
                    }
                }
                quick_xml::events::Event::Eof => {
                    return Err(ApiError::internal("unexpected end of xml-rpc response"));
                }
                _ => {}
            }
        }
    }

    fn read_text_until(&mut self, tag: &str) -> Result<String, ApiError> {
        let mut text = String::new();
        loop {
            let event = self.read_event()?;
            match event {
                quick_xml::events::Event::Text(t) => {
                    let decoded = t.xml_content(XmlVersion::Implicit1_0).into_owned();
                    text.push_str(&decoded);
                }
                quick_xml::events::Event::CData(t) => {
                    text.push_str(&t.xml_content(XmlVersion::Implicit1_0));
                }
                quick_xml::events::Event::GeneralRef(t) => {
                    if let Ok(Some(ch)) = t.resolve_char_ref() {
                        text.push(ch);
                    } else if let Some(resolved) =
                        quick_xml::escape::resolve_predefined_entity(t.as_ref())
                    {
                        text.push_str(resolved);
                    }
                }
                quick_xml::events::Event::End(e) => {
                    if e.name().as_ref() == tag {
                        return Ok(text);
                    }
                }
                quick_xml::events::Event::Eof => {
                    return Err(ApiError::internal("unexpected end of xml-rpc response"));
                }
                _ => {}
            }
        }
    }

    fn expect_end_tag(&mut self, tag: &str) -> Result<(), ApiError> {
        loop {
            let event = self.read_event()?;
            match event {
                quick_xml::events::Event::End(e) => {
                    if e.name().as_ref() == tag {
                        return Ok(());
                    }
                    return Err(ApiError::internal(format!(
                        "expected end tag '{}', got '{}'",
                        tag,
                        e.name().as_ref()
                    )));
                }
                quick_xml::events::Event::Text(t) => {
                    let decoded = t.xml_content(XmlVersion::Implicit1_0);
                    if decoded.trim().is_empty() {
                        continue;
                    }
                    return Err(ApiError::internal(format!(
                        "expected end tag '{}', found text",
                        tag
                    )));
                }
                quick_xml::events::Event::Eof => {
                    return Err(ApiError::internal("unexpected end of xml-rpc response"));
                }
                _ => {
                    return Err(ApiError::internal(format!("expected end tag '{}'", tag)));
                }
            }
        }
    }

    fn skip_until(&mut self, tag: &str) -> Result<(), ApiError> {
        let mut depth = 0usize;
        loop {
            let event = self.read_event()?;
            match event {
                quick_xml::events::Event::Start(e) => {
                    if e.name().as_ref() == tag {
                        depth += 1;
                    }
                }
                quick_xml::events::Event::End(e) => {
                    if e.name().as_ref() == tag {
                        if depth == 0 {
                            return Ok(());
                        }
                        depth -= 1;
                    }
                }
                quick_xml::events::Event::Eof => {
                    return Err(ApiError::internal("unexpected end of xml-rpc response"));
                }
                _ => {}
            }
        }
    }

    fn read_event(&mut self) -> Result<quick_xml::events::Event<'_>, ApiError> {
        self.reader
            .read_event()
            .map_err(|e| ApiError::internal(format!("bad xml-rpc response: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multi_param_response_with_entities() {
        let xml = "<methodResponse><params>\
                   <param><value><string>step&gt; done &amp; ok &#65;</string></value></param>\
                   <param><value><int>1234</int></value></param>\
                   <param><value><boolean>0</boolean></value></param>\
                   </params></methodResponse>";
        let value = parse_method_response(xml).unwrap();
        let params = value.as_array().unwrap();
        assert_eq!(params.len(), 3);
        assert_eq!(params[0], Value::String("step> done & ok A".into()));
        assert_eq!(params[1], Value::Number(1234.into()));
        assert_eq!(params[2], Value::Bool(false));
    }

    #[test]
    fn parses_fault() {
        let xml = "<methodResponse><fault><value><struct>\
                   <member><name>faultCode</name><value><int>-32500</int></value></member>\
                   <member><name>faultString</name><value><string>boom</string></value></member>\
                   </struct></value></fault></methodResponse>";
        let err = parse_method_response(xml).unwrap_err();
        assert!(err.message.contains("boom") || err.message.contains("-32500"));
    }

    #[test]
    fn parses_array_of_structs() {
        let xml = "<methodResponse><params><param><value><array><data>\
                   <value><struct>\
                   <member><name>name</name><value><string>a</string></value></member>\
                   <member><name>state</name><value><int>20</int></value></member>\
                   </struct></value>\
                   </data></array></value></param></params></methodResponse>";
        let value = parse_method_response(xml).unwrap();
        let params = value.as_array().unwrap();
        assert_eq!(params.len(), 1);
        let arr = params[0].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(string_field(&arr[0], "name").as_deref(), Some("a"));
        assert_eq!(number_field(&arr[0], "state"), Some(20));
    }
}
