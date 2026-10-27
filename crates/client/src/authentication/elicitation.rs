//! Renderable ACP elicitation requests and validation of user-selected responses.
//!
//! https://agentclientprotocol.com/protocol/v1/elicitation

use std::collections::BTreeSet;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use time::{Date, OffsetDateTime, format_description::well_known::Rfc3339};

use super::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub message: String,
    pub scope: Value,
    pub raw: Value,
    pub mode: Mode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Mode {
    Form(Form),
    Url(Url),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Url {
    pub url: String,
    pub host: String,
    pub elicitation_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Form {
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub title: String,
    pub description: Option<String>,
    pub required: bool,
    pub kind: Kind,
    pub default: Option<Value>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    String,
    Number,
    Integer,
    Boolean,
    Enum { options: Vec<Choice> },
    MultiSelect { options: Vec<Choice> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub value: String,
    pub title: String,
    pub description: Option<String>,
}

impl Form {
    /// Return declared defaults that satisfy their property schemas.
    #[must_use]
    pub fn defaults(&self) -> Map<String, Value> {
        self.fields
            .iter()
            .filter_map(|field| {
                field
                    .default
                    .as_ref()
                    .map(|value| (field.name.clone(), value.clone()))
            })
            .collect()
    }

    /// Validate a flat submitted form against the request's supported schema.
    ///
    /// # Errors
    /// Returns an error for missing required fields, undeclared fields, incorrect
    /// types, or values outside the declared constraints.
    pub fn validate(&self, content: &Map<String, Value>) -> Result<(), Error> {
        for name in content.keys() {
            if !self.fields.iter().any(|field| field.name == *name) {
                return Err(response(format!("未声明的表单字段：{name}")));
            }
        }
        for field in &self.fields {
            match content.get(&field.name) {
                Some(value) => field.validate(value)?,
                None if field.required => {
                    return Err(response(format!("请填写 {}", field.title)));
                }
                None => {}
            }
        }
        Ok(())
    }
}

impl Field {
    /// Validate one submitted JSON value against this property schema.
    ///
    /// # Errors
    /// Returns an error for an incorrect type, invalid enum selection, or a value
    /// outside the declared constraints.
    pub fn validate(&self, value: &Value) -> Result<(), Error> {
        self.validate_value(value)
            .map_err(|reason| response(format!("{}：{reason}", self.title)))
    }

    /// # Errors
    /// Returns a field-local reason for invalid schema, type, selection, or constraints.
    fn validate_value(&self, value: &Value) -> Result<(), String> {
        let schema = self
            .raw
            .as_object()
            .ok_or_else(|| "字段 schema 无效".to_owned())?;
        match &self.kind {
            Kind::String | Kind::Enum { .. } => {
                let value = value.as_str().ok_or_else(|| "请输入文本".to_owned())?;
                validate_string(schema, value)?;
                if let Kind::Enum { options } = &self.kind
                    && !options.iter().any(|choice| choice.value == value)
                {
                    return Err("请选择已提供的选项".into());
                }
            }
            Kind::Number => {
                let value = value.as_f64().ok_or_else(|| "请输入数字".to_owned())?;
                validate_number(schema, value)?;
            }
            Kind::Integer => {
                let value = value.as_i64().ok_or_else(|| "请输入整数".to_owned())?;
                if schema
                    .get("minimum")
                    .and_then(Value::as_i64)
                    .is_some_and(|min| value < min)
                    || schema
                        .get("maximum")
                        .and_then(Value::as_i64)
                        .is_some_and(|max| value > max)
                {
                    return Err("数值超出允许范围".into());
                }
            }
            Kind::Boolean if !value.is_boolean() => return Err("请选择开关状态".into()),
            Kind::Boolean => {}
            Kind::MultiSelect { options } => {
                let values = value
                    .as_array()
                    .ok_or_else(|| "请选择多个选项".to_owned())?;
                let mut unique = BTreeSet::new();
                for value in values {
                    let value = value.as_str().ok_or_else(|| "选项必须是文本".to_owned())?;
                    if !options.iter().any(|choice| choice.value == value) || !unique.insert(value)
                    {
                        return Err("请选择互不重复的已提供选项".into());
                    }
                }
                validate_count(schema, "minItems", "maxItems", values.len() as u64)?;
            }
        }
        Ok(())
    }
}

/// Parse a raw `elicitation/create` request into controls the client can render.
/// The original payload and field schemas are retained without normalization loss.
///
/// # Errors
/// Returns an error for unsupported modes, malformed or nested form schemas,
/// explicit sensitive input markers, and URLs outside HTTP/HTTPS.
pub fn parse_request(raw: &Value) -> Result<Request, Error> {
    let object = raw
        .as_object()
        .ok_or_else(|| elicitation("请求必须是 JSON 对象"))?;
    let message = string(object, "message")?.to_owned();
    let scope = parse_scope(object)?;
    let mode = match string(object, "mode")? {
        "form" => {
            reject_sensitive(object)?;
            Mode::Form(parse_form(
                object
                    .get("requestedSchema")
                    .ok_or_else(|| elicitation("缺少 requestedSchema"))?,
            )?)
        }
        "url" => Mode::Url(parse_url(object)?),
        mode => return Err(elicitation(format!("暂不支持交互类型 {mode}"))),
    };
    Ok(Request {
        message,
        scope,
        raw: raw.clone(),
        mode,
    })
}

/// Validate the response before forwarding it to the requesting ACP connection.
///
/// # Errors
/// Returns an error for unknown actions, invalid form content, or content supplied
/// with an accepted URL interaction.
pub fn validate_response(request: &Request, raw: &Value) -> Result<(), Error> {
    let object = raw
        .as_object()
        .ok_or_else(|| response("响应必须是 JSON 对象"))?;
    match object.get("action").and_then(Value::as_str) {
        Some("decline" | "cancel") => Ok(()),
        Some("accept") => match &request.mode {
            Mode::Form(form) => {
                let empty = Map::new();
                let content = match object.get("content") {
                    None | Some(Value::Null) => &empty,
                    Some(Value::Object(content)) => content,
                    Some(_) => return Err(response("表单内容必须是 JSON 对象")),
                };
                form.validate(content)
            }
            Mode::Url(_) => {
                if object.get("content").is_some_and(|value| !value.is_null()) {
                    return Err(response("外部授权应通过浏览器完成"));
                }
                Ok(())
            }
        },
        _ => Err(response("请选择继续、拒绝或取消")),
    }
}

fn parse_scope(object: &Map<String, Value>) -> Result<Value, Error> {
    let mut scope = Map::new();
    match (object.get("sessionId"), object.get("requestId")) {
        (Some(Value::String(id)), None) if !id.is_empty() => {
            scope.insert("sessionId".into(), Value::String(id.clone()));
            if let Some(tool) = object.get("toolCallId") {
                let tool = tool
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| elicitation("toolCallId 必须是非空字符串"))?;
                scope.insert("toolCallId".into(), Value::String(tool.into()));
            }
        }
        (None, Some(id)) if id.is_string() || id.as_i64().is_some() => {
            if object.contains_key("toolCallId") {
                return Err(elicitation("toolCallId 需要 sessionId"));
            }
            scope.insert("requestId".into(), id.clone());
        }
        _ => return Err(elicitation("请求必须指定单一有效的 sessionId 或 requestId")),
    }
    Ok(Value::Object(scope))
}

fn parse_url(object: &Map<String, Value>) -> Result<Url, Error> {
    let elicitation_id = string(object, "elicitationId")?.to_owned();
    if elicitation_id.is_empty() {
        return Err(elicitation("elicitationId 必须是非空字符串"));
    }
    let raw_url = string(object, "url")?;
    if raw_url.chars().any(char::is_control) || raw_url.trim() != raw_url {
        return Err(elicitation("授权网址包含无效字符"));
    }
    let url = reqwest::Url::parse(raw_url).map_err(|_| elicitation("授权网址无效"))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(elicitation(
            "授权网址需要使用 HTTP 或 HTTPS，且应通过页面完成认证",
        ));
    }
    let host = url
        .host_str()
        .filter(|host| !host.is_empty())
        .ok_or_else(|| elicitation("授权网址缺少主机名"))?
        .to_owned();
    Ok(Url {
        url: raw_url.into(),
        host,
        elicitation_id,
    })
}

fn parse_form(raw: &Value) -> Result<Form, Error> {
    let schema = raw
        .as_object()
        .ok_or_else(|| elicitation("requestedSchema 必须是 JSON 对象"))?;
    reject_sensitive(schema)?;
    reject_constraints(schema)?;
    applicable_constraints(
        schema,
        &["type", "properties", "required", "additionalProperties"],
    )?;
    if schema
        .get("additionalProperties")
        .is_some_and(|value| value != &Value::Bool(false))
    {
        return Err(elicitation("表单需要明确声明全部字段"));
    }
    if schema
        .get("type")
        .is_some_and(|kind| kind.as_str() != Some("object"))
    {
        return Err(elicitation("表单 schema 必须使用 object 类型"));
    }
    let empty = Map::new();
    let properties = match schema.get("properties") {
        None => &empty,
        Some(Value::Object(properties)) => properties,
        Some(_) => return Err(elicitation("表单 properties 必须是 JSON 对象")),
    };
    let required = string_array(schema.get("required"), "required")?
        .into_iter()
        .collect::<BTreeSet<_>>();
    if required.iter().any(|name| !properties.contains_key(name)) {
        return Err(elicitation("required 包含未声明的表单字段"));
    }
    let fields = properties
        .iter()
        .map(|(name, property)| parse_field(name, property, required.contains(name)))
        .collect::<Result<_, _>>()?;
    Ok(Form { fields })
}

fn parse_field(name: &str, raw: &Value, required: bool) -> Result<Field, Error> {
    let schema = raw
        .as_object()
        .ok_or_else(|| elicitation(format!("字段 {name} 的 schema 必须是对象")))?;
    reject_sensitive(schema)?;
    reject_constraints(schema)?;
    let kind = match string(schema, "type")? {
        "string" => {
            applicable_constraints(
                schema,
                &[
                    "type",
                    "minLength",
                    "maxLength",
                    "pattern",
                    "format",
                    "enum",
                    "oneOf",
                ],
            )?;
            ordered_unsigned(schema, "minLength", "maxLength")?;
            if let Some(pattern) = optional_string(schema, "pattern")? {
                Regex::new(pattern)
                    .map_err(|_| elicitation(format!("字段 {name} 的 pattern 暂不支持")))?;
            }
            if let Some(format) = optional_string(schema, "format")?
                && !matches!(format, "email" | "uri" | "date" | "date-time")
            {
                return Err(elicitation(format!(
                    "字段 {name} 的 format {format} 暂不支持"
                )));
            }
            match enum_choices(schema, "oneOf")? {
                Some(options) => Kind::Enum { options },
                None => Kind::String,
            }
        }
        "number" => {
            applicable_constraints(schema, &["type", "minimum", "maximum"])?;
            numeric_bounds(schema, false)?;
            Kind::Number
        }
        "integer" => {
            applicable_constraints(schema, &["type", "minimum", "maximum"])?;
            numeric_bounds(schema, true)?;
            Kind::Integer
        }
        "boolean" => {
            applicable_constraints(schema, &["type"])?;
            Kind::Boolean
        }
        "array" => {
            applicable_constraints(
                schema,
                &["type", "items", "minItems", "maxItems", "uniqueItems"],
            )?;
            if schema
                .get("uniqueItems")
                .is_some_and(|value| !value.is_boolean())
            {
                return Err(elicitation("uniqueItems 必须是布尔值"));
            }
            ordered_unsigned(schema, "minItems", "maxItems")?;
            let items = schema
                .get("items")
                .and_then(Value::as_object)
                .ok_or_else(|| elicitation("多选字段需要 items 对象"))?;
            reject_sensitive(items)?;
            reject_constraints(items)?;
            applicable_constraints(items, &["type", "enum", "anyOf"])?;
            if items
                .get("type")
                .is_some_and(|kind| kind.as_str() != Some("string"))
            {
                return Err(elicitation("多选字段的选项需要 string 类型"));
            }
            let options = enum_choices(items, "anyOf")?
                .ok_or_else(|| elicitation("多选字段需要声明 enum 或 anyOf 选项"))?;
            Kind::MultiSelect { options }
        }
        kind => return Err(elicitation(format!("字段 {name} 的类型 {kind} 暂不支持"))),
    };
    let mut field = Field {
        name: name.into(),
        title: schema
            .get("title")
            .and_then(Value::as_str)
            .filter(|title| !title.is_empty())
            .unwrap_or(name)
            .into(),
        description: schema
            .get("description")
            .and_then(Value::as_str)
            .map(String::from),
        required,
        kind,
        default: None,
        raw: raw.clone(),
    };
    if let Some(default) = schema.get("default")
        && field.validate(default).is_ok()
    {
        field.default = Some(default.clone());
    }
    Ok(field)
}

fn enum_choices(schema: &Map<String, Value>, titled: &str) -> Result<Option<Vec<Choice>>, Error> {
    let plain = schema.get("enum").filter(|value| !value.is_null());
    let labels = schema.get(titled).filter(|value| !value.is_null());
    if plain.is_some() && labels.is_some() {
        return Err(elicitation("枚举只能使用一种选项格式"));
    }
    let options = if let Some(plain) = plain {
        string_array(Some(plain), "enum")?
            .into_iter()
            .map(|value| Choice {
                title: value.clone(),
                value,
                description: None,
            })
            .collect::<Vec<_>>()
    } else if let Some(Value::Array(labels)) = labels {
        labels
            .iter()
            .map(|label| {
                let option = label
                    .as_object()
                    .ok_or_else(|| elicitation("枚举选项必须是对象"))?;
                reject_sensitive(option)?;
                reject_constraints(option)?;
                applicable_constraints(option, &["type", "const"])?;
                if option
                    .get("type")
                    .is_some_and(|kind| kind.as_str() != Some("string"))
                {
                    return Err(elicitation("枚举选项必须使用 string 类型"));
                }
                Ok(Choice {
                    value: string(option, "const")?.into(),
                    title: string(option, "title")?.into(),
                    description: option
                        .get("description")
                        .and_then(Value::as_str)
                        .map(String::from),
                })
            })
            .collect::<Result<_, _>>()?
    } else if labels.is_some() {
        return Err(elicitation("枚举选项必须是数组"));
    } else {
        return Ok(None);
    };
    let mut values = BTreeSet::new();
    if options.is_empty() || options.iter().any(|option| !values.insert(&option.value)) {
        return Err(elicitation("枚举选项需要非空且互不重复"));
    }
    Ok(Some(options))
}

fn reject_sensitive(schema: &Map<String, Value>) -> Result<(), Error> {
    // Explicit markers only: names and descriptions cannot determine credential intent.
    let marked = schema.get("writeOnly").and_then(Value::as_bool) == Some(true)
        || schema.get("x-sensitive").and_then(Value::as_bool) == Some(true)
        || schema.get("format").and_then(Value::as_str) == Some("password")
        || schema.get("_meta").is_some_and(|meta| {
            ["sensitive", "secret", "bas/sensitive"]
                .iter()
                .any(|key| meta.get(*key).and_then(Value::as_bool) == Some(true))
        });
    if marked {
        return Err(elicitation(
            "此表单要求敏感信息，请由 Agent 使用外部授权流程",
        ));
    }
    Ok(())
}

fn reject_constraints(schema: &Map<String, Value>) -> Result<(), Error> {
    const UNSUPPORTED: &[&str] = &[
        "$ref",
        "$dynamicRef",
        "allOf",
        "not",
        "if",
        "then",
        "else",
        "dependentSchemas",
        "dependentRequired",
        "patternProperties",
        "unevaluatedProperties",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "multipleOf",
        "contains",
        "prefixItems",
        "unevaluatedItems",
    ];
    if let Some(key) = UNSUPPORTED.iter().find(|key| schema.contains_key(**key)) {
        return Err(elicitation(format!("暂不支持表单约束 {key}")));
    }
    Ok(())
}

fn applicable_constraints(schema: &Map<String, Value>, supported: &[&str]) -> Result<(), Error> {
    const CONSTRAINTS: &[&str] = &[
        "type",
        "properties",
        "required",
        "additionalProperties",
        "enum",
        "const",
        "oneOf",
        "anyOf",
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
        "pattern",
        "format",
        "items",
        "minItems",
        "maxItems",
        "uniqueItems",
        "minProperties",
        "maxProperties",
        "propertyNames",
        "contentEncoding",
        "contentMediaType",
        "contentSchema",
    ];
    if let Some(key) = CONSTRAINTS
        .iter()
        .find(|key| schema.contains_key(**key) && !supported.contains(key))
    {
        return Err(elicitation(format!("此字段类型暂不支持约束 {key}")));
    }
    Ok(())
}

fn validate_string(schema: &Map<String, Value>, value: &str) -> Result<(), String> {
    validate_count(
        schema,
        "minLength",
        "maxLength",
        value.chars().count() as u64,
    )?;
    if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
        let pattern = Regex::new(pattern).map_err(|_| "文本规则暂不支持".to_owned())?;
        if !pattern.is_match(value) {
            return Err("文本与要求的格式不匹配".into());
        }
    }
    if let Some(format) = schema.get("format").and_then(Value::as_str) {
        let valid = match format {
            "email" => valid_email(value),
            "uri" => reqwest::Url::parse(value).is_ok(),
            "date" => valid_date(value),
            "date-time" => valid_date_time(value),
            _ => false,
        };
        if !valid {
            return Err(format!("请输入有效的 {format} 格式"));
        }
    }
    Ok(())
}

fn validate_count(
    schema: &Map<String, Value>,
    min: &str,
    max: &str,
    value: u64,
) -> Result<(), String> {
    if schema
        .get(min)
        .and_then(Value::as_u64)
        .is_some_and(|min| value < min)
        || schema
            .get(max)
            .and_then(Value::as_u64)
            .is_some_and(|max| value > max)
    {
        return Err("长度或选择数量超出允许范围".into());
    }
    Ok(())
}

fn validate_number(schema: &Map<String, Value>, value: f64) -> Result<(), String> {
    if !value.is_finite()
        || schema
            .get("minimum")
            .and_then(Value::as_f64)
            .is_some_and(|min| value < min)
        || schema
            .get("maximum")
            .and_then(Value::as_f64)
            .is_some_and(|max| value > max)
    {
        return Err("数值超出允许范围".into());
    }
    Ok(())
}

fn valid_email(value: &str) -> bool {
    email_address::EmailAddress::parse_with_options(
        value,
        email_address::Options::default().without_display_text(),
    )
    .is_ok()
}

fn valid_date(value: &str) -> bool {
    value.len() == 10
        && Date::parse(
            value,
            time::macros::format_description!("[year]-[month]-[day]"),
        )
        .is_ok()
}

fn valid_date_time(value: &str) -> bool {
    OffsetDateTime::parse(value, &Rfc3339).is_ok()
}

fn numeric_bounds(schema: &Map<String, Value>, integer: bool) -> Result<(), Error> {
    for key in ["minimum", "maximum"] {
        if let Some(value) = schema.get(key).filter(|value| !value.is_null())
            && (value.as_f64().is_none() || integer && value.as_i64().is_none())
        {
            return Err(elicitation(format!("{key} 必须是有效数字")));
        }
    }
    let reversed = if integer {
        matches!(
            (
                schema.get("minimum").and_then(Value::as_i64),
                schema.get("maximum").and_then(Value::as_i64),
            ),
            (Some(min), Some(max)) if min > max
        )
    } else {
        matches!(
            (
                schema.get("minimum").and_then(Value::as_f64),
                schema.get("maximum").and_then(Value::as_f64),
            ),
            (Some(min), Some(max)) if min > max
        )
    };
    if reversed {
        return Err(elicitation("minimum 大于 maximum"));
    }
    Ok(())
}

fn unsigned_bound(schema: &Map<String, Value>, key: &str) -> Result<Option<u64>, Error> {
    match schema.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| elicitation(format!("{key} 必须是非负整数"))),
    }
}

fn ordered_unsigned(schema: &Map<String, Value>, min: &str, max: &str) -> Result<(), Error> {
    if let (Some(minimum), Some(maximum)) =
        (unsigned_bound(schema, min)?, unsigned_bound(schema, max)?)
        && minimum > maximum
    {
        return Err(elicitation(format!("{min} 大于 {max}")));
    }
    Ok(())
}

fn string_array(raw: Option<&Value>, name: &str) -> Result<Vec<String>, Error> {
    match raw {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(String::from)
                    .ok_or_else(|| elicitation(format!("{name} 必须由字符串组成")))
            })
            .collect(),
        Some(_) => Err(elicitation(format!("{name} 必须是数组"))),
    }
}

fn string<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, Error> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| elicitation(format!("{key} 必须是字符串")))
}

fn optional_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a str>, Error> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(elicitation(format!("{key} 必须是字符串"))),
    }
}

fn elicitation(reason: impl Into<String>) -> Error {
    Error::Elicitation(reason.into())
}

fn response(reason: impl Into<String>) -> Error {
    Error::Response(reason.into())
}

#[cfg(test)]
mod tests;
