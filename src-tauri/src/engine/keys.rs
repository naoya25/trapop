use std::io::Write;
use std::process::{Command, Stdio};

pub fn resolve_api_key(service: &str, env_var: &str) -> Option<String> {
    keychain_api_key(service).or_else(|| {
        std::env::var(env_var)
            .ok()
            .filter(|v| !v.trim().is_empty())
    })
}

pub fn keychain_api_key(service: &str) -> Option<String> {
    let output = Command::new("security")
        .args(["find-generic-password", "-s", service, "-w"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let key = String::from_utf8(output.stdout).ok()?;
    let trimmed = key.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

// 存在確認だけなら -w(平文取り出し)を使わない。読み出し ACL に触れず、
// 鍵をプロセスメモリに載せずに済む。
pub fn has_stored_key(service: &str) -> bool {
    Command::new("security")
        .args(["find-generic-password", "-s", service])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

// キーを argv に載せると同一ユーザーの ps から平文で見えるため、security の
// 対話モード(-i)に stdin でコマンドごと渡す。
pub fn store_api_key(service: &str, key: &str) -> Result<(), String> {
    if key
        .chars()
        .any(|c| c == '"' || c == '\\' || c.is_control())
    {
        return Err("APIキーに使用できない文字が含まれています".to_string());
    }

    let mut child = Command::new("security")
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;

    let command = format!(
        "add-generic-password -U -a trapop -s \"{service}\" -w \"{key}\"\n"
    );
    child
        .stdin
        .as_mut()
        .ok_or_else(|| "security コマンドの stdin を開けませんでした".to_string())?
        .write_all(command.as_bytes())
        .map_err(|e| e.to_string())?;

    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("Keychain へのAPIキー保存に失敗しました".to_string())
    }
}
