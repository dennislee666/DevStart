//! Thin CLI and MCP stdio adapters for the same local API.
use crate::{api, api_transport};
use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};

pub fn cli(args: &[String]) -> Result<(), String> {
    let root = api_transport::workspace()?;
    let method = args
        .get(2)
        .ok_or("用法: dev-start --api METHOD [JSON|@FILE|-]")?;
    let raw = match args.get(3).map(String::as_str) {
        None => "{}".to_string(),
        Some("-") => {
            let mut text = String::new();
            std::io::stdin()
                .take(api_transport::REQUEST_LIMIT + 1)
                .read_to_string(&mut text)
                .map_err(|e| e.to_string())?;
            text
        }
        Some(path) if path.starts_with('@') => {
            let mut text = String::new();
            std::fs::File::open(&path[1..])
                .map_err(|e| e.to_string())?
                .take(api_transport::REQUEST_LIMIT + 1)
                .read_to_string(&mut text)
                .map_err(|e| e.to_string())?;
            text
        }
        Some(raw) => raw.to_string(),
    };
    if raw.len() as u64 > api_transport::REQUEST_LIMIT {
        return Err("请求超限".into());
    }
    let value = api_transport::call(
        &root,
        method,
        serde_json::from_str(&raw).map_err(|e| e.to_string())?,
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
    );
    if value.get("error").is_some() {
        return Err(value["error"]["message"]
            .as_str()
            .unwrap_or("API失败")
            .into());
    }
    Ok(())
}
pub fn follow(args: &[String]) -> Result<(), String> {
    let id: u64 = args
        .get(2)
        .ok_or("用法: dev-start --logs RUN_ID [OFFSET]")?
        .parse::<u64>()
        .map_err(|e| e.to_string())?;
    let mut offset = args
        .get(3)
        .map(|v| v.parse::<u64>())
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or(0);
    let root = api_transport::workspace()?;
    loop {
        let value = api_transport::call(
            &root,
            "logs.read",
            json!({"run_id":id,"offset":offset,"limit":65536}),
        )?;
        if let Some(e) = value.get("error") {
            return Err(e.to_string());
        }
        let r = &value["result"];
        if r["reset"] == true {
            eprintln!("[dev start] 日志保留范围已变化，从当前最早可读位置继续");
        }
        let bytes: Vec<u8> =
            serde_json::from_value(r["bytes"].clone()).map_err(|e| e.to_string())?;
        std::io::stdout()
            .write_all(&bytes)
            .and_then(|_| std::io::stdout().flush())
            .map_err(|e| e.to_string())?;
        offset = r["next_offset"].as_u64().ok_or("日志游标缺失")?;
        if r["eof"] == true {
            if r["active"] == false {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    }
    Ok(())
}
// Generate MCP discovery from the API contract to prevent parameter drift.
fn tools_catalog() -> Vec<Value> {
    let contracts: Value = serde_json::from_str(include_str!("../docs/api-schema.json"))
        .expect("bundled API contract");
    let methods: Vec<_> = api::METHODS
        .iter()
        .filter(|m| !m.starts_with("git."))
        .copied()
        .collect();
    let mut tools = vec![
        json!({"name":"dev_start","description":"Compatibility dispatcher for non-Git DevStart operations. Prefer the individually typed devstart_* tools. Read devstart_api_docs first.","inputSchema":{"type":"object","properties":{"method":{"type":"string","enum":methods},"params":{"type":"object"}},"required":["method","params"],"additionalProperties":false}}),
    ];
    tools.extend(methods.iter().map(|method| {
        json!({
            "name":format!("devstart_{}",method.replace('.',"_")),
            "description":contracts["methods"][*method]["description"],
            "inputSchema":contracts["methods"][*method]["params"]
        })
    }));
    tools
}
fn tool_method(name: &str) -> Option<&'static str> {
    api::METHODS
        .iter()
        .copied()
        .filter(|m| !m.starts_with("git."))
        .find(|m| format!("devstart_{}", m.replace('.', "_")) == name)
}
fn mcp_result(request: &Value, root: &std::path::Path, initialized: &mut bool) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request["method"].as_str().unwrap_or("");
    if request["jsonrpc"] != "2.0" {
        return api::error(id, -32600, "Invalid JSON-RPC request");
    }
    let result = match method {
        "initialize" => {
            *initialized = true;
            json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{},"resources":{}},"serverInfo":{"name":"dev-start","version":crate::app_info::VERSION},
                "instructions":"Call api.describe first and read docs/api.md. Configuration never starts scripts implicitly. Use explicit runs.start/runs.restart only for the requested launch. Disabled deployment entries remain disabled. logs.read returns bounded byte-cursor pages."})
        }
        "ping" => json!({}),
        _ if !*initialized => return api::error(id, -32000, "Initialize first"),
        "tools/list" => json!({"tools":tools_catalog()}),
        "resources/list" => {
            json!({"resources":[{"uri":"devstart://docs/api","name":"DevStart API and MCP manual","mimeType":"text/markdown"}]})
        }
        "resources/read" => {
            if request["params"]["uri"] != "devstart://docs/api" {
                return api::error(id, -32602, "Unknown resource");
            }
            json!({"contents":[{"uri":"devstart://docs/api","mimeType":"text/markdown","text":include_str!("../docs/api.md")} ]})
        }
        "tools/call" => {
            let params = &request["params"];
            let name = params["name"].as_str().unwrap_or("");
            let args = &params["arguments"];
            let (method, arguments) = if name == "dev_start" {
                let Some(method) = args["method"]
                    .as_str()
                    .filter(|v| api::METHODS.contains(v) && !v.starts_with("git."))
                else {
                    return api::error(id, -32602, "Unknown or out-of-scope API method");
                };
                (method, &args["params"])
            } else {
                let Some(method) = tool_method(name) else {
                    return api::error(id, -32602, "Unknown tool");
                };
                (method, args)
            };
            if !arguments.is_object() {
                return api::error(id, -32602, "arguments must be an object");
            }
            match api_transport::call(root, method, arguments.clone()) {
                Ok(value) => {
                    json!({"content":[{"type":"text","text":serde_json::to_string(&value).unwrap_or_default()}],"isError":value.get("error").is_some()})
                }
                Err(e) => json!({"content":[{"type":"text","text":e}],"isError":true}),
            }
        }
        _ => return api::error(id, -32601, "Method not found"),
    };
    json!({"jsonrpc":"2.0","id":id,"result":result})
}
pub fn mcp() -> Result<(), String> {
    let root = api_transport::workspace()?;
    let mut input = std::io::stdin().lock();
    let mut initialized = false;
    loop {
        let mut bytes = Vec::new();
        let count = (&mut input)
            .take(api_transport::REQUEST_LIMIT + 1)
            .read_until(b'\n', &mut bytes)
            .map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        if count as u64 > api_transport::REQUEST_LIMIT || !bytes.ends_with(b"\n") {
            return Err("MCP请求超限或缺少换行".into());
        }
        let response = match serde_json::from_slice::<Value>(&bytes) {
            Ok(request) => {
                if request.get("id").is_none() {
                    continue;
                }
                mcp_result(&request, &root, &mut initialized)
            }
            Err(e) => api::error(Value::Null, -32700, &e.to_string()),
        };
        let mut out = std::io::stdout().lock();
        serde_json::to_writer(&mut out, &response).map_err(|e| e.to_string())?;
        out.write_all(b"\n")
            .and_then(|_| out.flush())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_tools_cover_every_non_git_contract_and_reject_git() {
        let tools = tools_catalog();
        let contracts: Value =
            serde_json::from_str(include_str!("../docs/api-schema.json")).unwrap();
        let mut names = std::collections::HashSet::new();
        for tool in &tools {
            assert!(names.insert(tool["name"].as_str().unwrap()));
        }
        for method in api::METHODS.iter().filter(|m| !m.starts_with("git.")) {
            let name = format!("devstart_{}", method.replace('.', "_"));
            let tool = tools.iter().find(|t| t["name"] == name).unwrap();
            assert_eq!(tool["inputSchema"], contracts["methods"][*method]["params"]);
            assert!(tool["description"].is_string(), "{method}");
            assert_eq!(tool_method(&name), Some(*method));
        }
        assert_eq!(tool_method("devstart_git_update"), None);
        let mut initialized = true;
        let reply = mcp_result(
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"dev_start","arguments":{"method":"git.update","params":{}}}}),
            std::path::Path::new("/does-not-exist"),
            &mut initialized,
        );
        assert_eq!(reply["error"]["code"], -32602);
        let docs = mcp_result(
            &json!({"jsonrpc":"2.0","id":2,"method":"resources/read","params":{"uri":"devstart://docs/api"}}),
            std::path::Path::new("/does-not-exist"),
            &mut initialized,
        );
        assert!(
            docs["result"]["contents"][0]["text"]
                .as_str()
                .unwrap()
                .contains("groups.create")
        );
    }
}
