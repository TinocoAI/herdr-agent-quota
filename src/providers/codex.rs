use crate::cache::CacheStore;
use crate::model::{Provider, ProviderSnapshot, UsageWindow, WindowKind};
use crate::providers::ProviderError;
use anyhow::{Context, Result};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

pub fn parse_rate_limits(
    value: &Value,
    fetched_at_unix: u64,
) -> std::result::Result<ProviderSnapshot, ProviderError> {
    let result = value.get("result").unwrap_or(value);
    let limits = result
        .get("rateLimits")
        .or_else(|| result.get("rate_limits"))
        .ok_or_else(|| ProviderError::UnsupportedResponse("missing rateLimits".to_string()))?;
    let objects = [limits.get("primary"), limits.get("secondary")]
        .into_iter()
        .flatten();
    let weekly = objects
        .filter_map(|candidate| {
            let duration = candidate
                .get("windowDurationMins")
                .or_else(|| candidate.get("window_duration_mins"))
                .and_then(Value::as_u64)?;
            if duration < 10_000 {
                return None;
            }
            let used = candidate
                .get("usedPercent")
                .or_else(|| candidate.get("used_percent"))
                .and_then(Value::as_f64)?;
            let reset = candidate
                .get("resetsAt")
                .or_else(|| candidate.get("resets_at"))
                .and_then(Value::as_str)
                .map(str::to_string);
            Some((used, reset))
        })
        .next()
        .ok_or_else(|| ProviderError::UnsupportedResponse("no seven-day rate limit".to_string()))?;
    let window = UsageWindow::new(WindowKind::Weekly, weekly.0, weekly.1)
        .map_err(|error| ProviderError::UnsupportedResponse(error.to_string()))?;
    Ok(ProviderSnapshot::new(
        Provider::Codex,
        vec![window],
        fetched_at_unix,
    ))
}

pub fn fetch() -> Result<ProviderSnapshot> {
    let executable = std::env::var_os("CODEX_BIN_PATH").unwrap_or_else(|| "codex".into());
    let mut command = Command::new(executable);
    command
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().context("start codex app-server")?;
    let mut input = child.stdin.take().context("open codex app-server stdin")?;
    let stdout = child
        .stdout
        .take()
        .context("open codex app-server stdout")?;
    let mut output = BufReader::new(stdout);
    let finished = Arc::new(AtomicBool::new(false));
    let timer_finished = Arc::clone(&finished);
    let pid = child.id();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(15));
        if !timer_finished.load(Ordering::Acquire) {
            unsafe {
                #[cfg(unix)]
                libc::killpg(pid as libc::pid_t, libc::SIGKILL);
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
        }
    });
    let result = fetch_from_process(&mut child, &mut input, &mut output);
    finished.store(true, Ordering::Release);
    terminate_process_tree(&mut child);
    let _ = child.wait();
    result
}

fn terminate_process_tree(child: &mut Child) {
    let pid = child.id();
    #[cfg(unix)]
    unsafe {
        let _ = libc::killpg(pid as libc::pid_t, libc::SIGKILL);
    }
    let _ = child.kill();
    #[cfg(unix)]
    {
        let _ = Command::new("pkill")
            .args(["-KILL", "-P", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn fetch_from_process(
    _child: &mut Child,
    input: &mut ChildStdin,
    output: &mut BufReader<impl std::io::Read>,
) -> Result<ProviderSnapshot> {
    write_rpc(
        input,
        1,
        "initialize",
        serde_json::json!({
            "clientInfo": {"name": "herdr-agent-quota", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {}
        }),
    )?;
    let _ = read_rpc(output, 1)?;
    write_notification(input, "initialized", serde_json::json!({}))?;

    write_rpc(input, 2, "account/read", serde_json::json!({}))?;
    let account = read_rpc(output, 2)?;
    if !account_is_chatgpt(&account) {
        anyhow::bail!(ProviderError::Unavailable(
            "Codex is using API-key auth, not a ChatGPT subscription".to_string()
        ));
    }

    write_rpc(input, 3, "account/rateLimits/read", serde_json::json!({}))?;
    let limits = read_rpc(output, 3)?;
    parse_rate_limits(&limits, CacheStore::now_unix()).map_err(anyhow::Error::from)
}

fn write_rpc(input: &mut ChildStdin, id: u64, method: &str, params: Value) -> Result<()> {
    let message = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    });
    writeln!(input, "{}", serde_json::to_string(&message)?)?;
    input.flush()?;
    Ok(())
}

fn write_notification(input: &mut ChildStdin, method: &str, params: Value) -> Result<()> {
    let message = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params
    });
    writeln!(input, "{}", serde_json::to_string(&message)?)?;
    input.flush()?;
    Ok(())
}

fn read_rpc(output: &mut BufReader<impl std::io::Read>, expected_id: u64) -> Result<Value> {
    let mut line = String::new();
    loop {
        line.clear();
        let count = output.read_line(&mut line)?;
        if count == 0 {
            anyhow::bail!("Codex app-server exited before response {expected_id}");
        }
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if value.get("id").and_then(Value::as_u64) != Some(expected_id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            anyhow::bail!("Codex app-server request failed: {error}");
        }
        return Ok(value);
    }
}

pub fn account_is_chatgpt(value: &Value) -> bool {
    let result = value.get("result").unwrap_or(value);
    let account = result.get("account").unwrap_or(result);
    let auth_mode = account
        .get("authMode")
        .or_else(|| account.get("auth_mode"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let account_type = account
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let plan = account
        .get("plan")
        .and_then(Value::as_str)
        .unwrap_or_default();
    [auth_mode, account_type, plan]
        .iter()
        .any(|value| value.to_ascii_lowercase().contains("chatgpt"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn selects_weekly_codex_window_by_duration_not_position() {
        let value = json!({
            "result": {"rateLimits": {
                "primary": {"usedPercent": 20.0, "windowDurationMins": 300, "resetsAt": "short"},
                "secondary": {"usedPercent": 61.0, "windowDurationMins": 10080, "resetsAt": "weekly"}
            }}
        });
        let snapshot = parse_rate_limits(&value, 1).unwrap();
        assert_eq!(snapshot.summary(), "week 39% left");
        assert_eq!(
            snapshot
                .window(WindowKind::Weekly)
                .unwrap()
                .resets_at
                .as_deref(),
            Some("weekly")
        );
    }

    #[test]
    fn rejects_codex_response_without_seven_day_window() {
        let value = json!({"result": {"rateLimits": {
            "primary": {"usedPercent": 20.0, "windowDurationMins": 300}
        }}});
        assert!(parse_rate_limits(&value, 1).is_err());
    }

    #[test]
    fn distinguishes_chatgpt_subscription_from_api_key() {
        assert!(account_is_chatgpt(
            &json!({"result": {"account": {"authMode": "chatgpt"}}})
        ));
        assert!(!account_is_chatgpt(
            &json!({"result": {"account": {"authMode": "api_key"}}})
        ));
    }
}
