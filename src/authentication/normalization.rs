use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use super::{Arguments, Error, Execution, Method, Source, Terminal};

const AUTH_REQUIRED: i64 = -32000;

/// Normalize authentication methods while retaining the complete descriptors.
///
/// An explicit `AUTH_REQUIRED.data.authMethods` list is the active list, including
/// an empty list. Its methods can only inherit the documented legacy terminal
/// semantics when the matching initializer descriptor proves the same operation.
///
/// # Errors
/// Returns an error if the selected method list or initialization container has an
/// invalid shape. Invalid individual descriptors remain visible as unsupported.
pub fn methods(initialization: &Value, error: Option<&Value>) -> Result<Vec<Method>, Error> {
    let initialization = initialization
        .as_object()
        .ok_or_else(|| Error::Descriptor("initialize result must be an object".into()))?;
    let initial = list(initialization.get("authMethods"))?;
    let error_methods = error
        .filter(|error| error.get("code").and_then(Value::as_i64) == Some(AUTH_REQUIRED))
        .and_then(|error| error.get("data"))
        .and_then(|data| data.get("authMethods"));
    let source = if error_methods.is_some() {
        Source::AuthenticationError
    } else {
        Source::Initialize
    };
    let active = if let Some(raw) = error_methods {
        list(Some(raw))?
    } else {
        initial
    };
    let mut methods = active
        .iter()
        .map(|raw| normalize(raw, &source, initial, initialization))
        .collect::<Vec<_>>();
    let mut ids = BTreeSet::new();
    let duplicates = methods
        .iter()
        .filter_map(|method| (!ids.insert(method.id.clone())).then_some(method.id.clone()))
        .collect::<BTreeSet<_>>();
    for method in &mut methods {
        if duplicates.contains(&method.id) {
            method.execution = unsupported("Agent 返回了重复的认证方法 ID");
        }
    }
    Ok(methods)
}

fn list(raw: Option<&Value>) -> Result<&[Value], Error> {
    match raw {
        None => Ok(&[]),
        Some(Value::Array(methods)) => Ok(methods),
        Some(_) => Err(Error::Descriptor("authMethods must be an array".into())),
    }
}

fn normalize(
    raw: &Value,
    source: &Source,
    initial: &[Value],
    initialization: &Map<String, Value>,
) -> Method {
    let id = raw.get("id").and_then(Value::as_str).unwrap_or_default();
    let name = raw
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .unwrap_or(if id.is_empty() { "认证方式" } else { id });
    let execution = match descriptor_execution(raw, source, initial, initialization) {
        Ok(execution) => execution,
        Err(reason) => unsupported(reason),
    };
    Method {
        id: id.into(),
        name: name.into(),
        description: raw
            .get("description")
            .and_then(Value::as_str)
            .map(String::from),
        raw: raw.clone(),
        source: source.clone(),
        execution,
    }
}

fn descriptor_execution(
    raw: &Value,
    source: &Source,
    initial: &[Value],
    initialization: &Map<String, Value>,
) -> Result<Execution, String> {
    let object = raw
        .as_object()
        .ok_or_else(|| "认证方法必须是 JSON 对象".to_owned())?;
    required_string(object, "id")?;
    required_string(object, "name")?;
    match object.get("type") {
        Some(Value::String(kind)) if kind == "agent" => Ok(Execution::Agent),
        Some(Value::String(kind)) if kind == "terminal" => {
            let standard = terminal(object, Arguments::Append, None)?;
            if *source == Source::AuthenticationError
                && let Some(legacy) = matching_legacy(raw, initial)
                && legacy.args == standard.args
                && legacy.env == standard.env
                && !object.contains_key("command")
            {
                return Ok(Execution::Terminal(legacy));
            }
            Ok(Execution::Terminal(standard))
        }
        Some(Value::String(kind)) => Err(format!("暂不支持认证类型 {kind}")),
        Some(_) => Err("认证类型 type 必须是字符串".into()),
        None => omitted_type(object, initialization),
    }
}

fn omitted_type(
    object: &Map<String, Value>,
    initialization: &Map<String, Value>,
) -> Result<Execution, String> {
    if let Some(legacy) = object
        .get("_meta")
        .and_then(|meta| meta.get("terminal-auth"))
    {
        return legacy_terminal(legacy).map(Execution::Terminal);
    }
    let Some(meta) = object.get("_meta").and_then(Value::as_object) else {
        return Ok(Execution::Agent);
    };
    if meta.get("type").and_then(Value::as_str) != Some("terminal") {
        return Ok(Execution::Agent);
    }
    // This nonstandard payload is verified against Qwen Code 0.25.0. Its args
    // target the interactive application, not the existing ACP-mode invocation.
    // https://github.com/QwenLM/qwen-code/blob/v0.25.0/packages/cli/src/acp-integration/authMethods.ts
    let info = initialization.get("agentInfo");
    if info
        .and_then(|info| info.get("name"))
        .and_then(Value::as_str)
        != Some("qwen-code")
        || info
            .and_then(|info| info.get("version"))
            .and_then(Value::as_str)
            != Some("0.25.0")
        || object.get("id").and_then(Value::as_str) != Some("openai")
    {
        return Err("此旧版终端认证描述尚未验证，请使用 Agent 的配置入口".into());
    }
    let descriptor = terminal(meta, Arguments::ReplaceApplication, None)?;
    if descriptor.args != ["--auth-type=openai"] || !descriptor.env.is_empty() {
        return Err("旧版终端认证参数与已验证的协议描述不一致".into());
    }
    Ok(Execution::Terminal(descriptor))
}

fn matching_legacy(raw: &Value, initial: &[Value]) -> Option<Terminal> {
    let id = raw.get("id")?.as_str()?;
    let mut matches = initial
        .iter()
        .filter(|method| method.get("id").and_then(Value::as_str) == Some(id));
    let method = matches.next()?;
    if matches.next().is_some() || method.get("type").is_some() {
        return None;
    }
    required_string(method.as_object()?, "name").ok()?;
    let nested = method.get("_meta")?.get("terminal-auth")?;
    legacy_terminal(nested).ok()
}

fn legacy_terminal(raw: &Value) -> Result<Terminal, String> {
    let object = raw
        .as_object()
        .ok_or_else(|| "terminal-auth 必须是 JSON 对象".to_owned())?;
    if let Some(kind) = object.get("type")
        && kind.as_str() != Some("terminal")
    {
        return Err("terminal-auth 的 type 与终端认证不一致".into());
    }
    let command = required_string(object, "command")?.to_owned();
    if command.contains('\0') {
        return Err("终端认证程序包含无效字符".into());
    }
    terminal(object, Arguments::ReplaceInvocation, Some(command))
}

fn terminal(
    object: &Map<String, Value>,
    arguments: Arguments,
    command: Option<String>,
) -> Result<Terminal, String> {
    let args = match object.get("args") {
        None => Vec::new(),
        Some(Value::Array(args)) => args
            .iter()
            .map(|arg| {
                arg.as_str()
                    .filter(|arg| !arg.contains('\0'))
                    .map(String::from)
                    .ok_or_else(|| "终端认证 args 必须由有效字符串组成".to_owned())
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("终端认证 args 必须是字符串数组".into()),
    };
    let env = match object.get("env") {
        None => BTreeMap::new(),
        Some(Value::Object(env)) => env
            .iter()
            .map(|(key, value)| {
                if key.is_empty() || key.contains(['\0', '=']) {
                    return Err("终端认证环境变量名称无效".to_owned());
                }
                let value = value
                    .as_str()
                    .filter(|value| !value.contains('\0'))
                    .ok_or_else(|| "终端认证环境变量值必须是有效字符串".to_owned())?;
                Ok((key.clone(), value.to_owned()))
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("终端认证 env 必须是字符串键值对象".into()),
    };
    Ok(Terminal {
        args,
        env,
        arguments,
        command,
    })
}

fn required_string<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("认证方法 {key} 必须是非空字符串"))
}

fn unsupported(reason: impl Into<String>) -> Execution {
    Execution::Unsupported {
        reason: reason.into(),
    }
}
