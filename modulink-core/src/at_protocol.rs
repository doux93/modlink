//! AT 协议基础：终结码判定、Hex 编解码。

/// 终结码判定：返回 Some 表示该行是一次 AT 命令交互的结束。
pub fn classify_final(line: &str) -> Option<String> {
    let t = line.trim();
    if t == "OK" || t == "ERROR" || t == "ABORTED" || t == "NO CARRIER" || t == "BUSY" {
        return Some(t.to_string());
    }
    if t.starts_with("+CME ERROR:") || t.starts_with("+CMS ERROR:") {
        return Some(t.to_string());
    }
    None
}

/// Extract the conventional response prefix from an AT command.
/// `AT+CEREG?` becomes `+CEREG`; basic commands such as `AT`/`ATI` have none.
pub fn response_prefix(command: &str) -> Option<String> {
    let command = command.trim().to_ascii_uppercase();
    let rest = command
        .strip_prefix("AT+")
        .or_else(|| command.strip_prefix("at+"))?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(format!("+{}", name.to_ascii_uppercase()))
    }
}

/// Best-effort separation of unsolicited result codes from a pending response.
/// A `+XXX` line whose prefix differs from the active command is treated as a
/// URC, while plain-text response lines remain associated with the command.
pub fn is_likely_urc(command: &str, line: &str) -> bool {
    let line = line.trim_start();
    if !(line.starts_with('+') || line.starts_with('*')) {
        return false;
    }
    if line.starts_with("+CME ERROR:") || line.starts_with("+CMS ERROR:") {
        return false;
    }
    match response_prefix(command) {
        Some(prefix) => !line.to_ascii_uppercase().starts_with(&prefix),
        None => true,
    }
}

pub fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !cleaned.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Hex 数据只能包含 0-9、A-F 和空白字符".into());
    }
    if cleaned.len() % 2 != 0 {
        return Err("Hex 数据长度必须为偶数".into());
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&cleaned[i..i + 2], 16)
                .map_err(|_| format!("非法 Hex: {}", &cleaned[i..i + 2]))
        })
        .collect()
}

pub fn split_at_arguments(command: &str) -> Vec<String> {
    let Some((_, arguments)) = command.split_once('=') else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for character in arguments.chars() {
        match character {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                result.push(current.trim().trim_matches('"').to_string());
                current.clear();
            }
            _ => current.push(character),
        }
    }
    result.push(current.trim().trim_matches('"').to_string());
    result
}

pub fn validate_at_command(cmd: &str) -> Result<(), String> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return Err("AT 指令不能为空".into());
    }
    if trimmed.len() > 4096 {
        return Err("单条 AT 指令超过 4096 字节".into());
    }
    if trimmed.contains(['\r', '\n', '\0']) {
        return Err("单条 AT 指令不能包含换行或 NUL 字符".into());
    }
    if !trimmed.to_ascii_uppercase().starts_with("AT") {
        return Err("统一 AT 执行通道只接受 AT 指令".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_command_validation_and_argument_parsing() {
        assert!(validate_at_command("AT+CSQ\r\nAT&F").is_err());
        assert!(validate_at_command("AT\0").is_err());
        assert!(validate_at_command("AT&F").is_ok());
        assert_eq!(
            split_at_arguments("AT+CFG=1,\"a,b\",2"),
            vec!["1", "a,b", "2"]
        );
    }

    #[test]
    fn hex_decode() {
        assert_eq!(decode_hex("1A 2b").unwrap(), vec![0x1a, 0x2b]);
        assert!(decode_hex("1").is_err());
        assert!(decode_hex("zz").is_err());
        for input in ["中0", "中文", "🙂", "ＡＢ", "0é0"] {
            assert!(decode_hex(input).is_err(), "{input}");
        }
        assert_eq!(decode_hex("00\tff\n7F").unwrap(), vec![0, 255, 127]);
    }

    #[test]
    fn final_codes() {
        assert_eq!(classify_final("OK"), Some("OK".into()));
        assert_eq!(classify_final(" ERROR "), Some("ERROR".into()));
        assert_eq!(
            classify_final("+CME ERROR: 148"),
            Some("+CME ERROR: 148".into())
        );
        assert_eq!(classify_final("+CSQ: 22,0"), None);
        assert_eq!(classify_final("AT+CSQ"), None);
    }

    #[test]
    fn separates_interleaved_urcs() {
        assert_eq!(response_prefix("AT+CEREG?"), Some("+CEREG".into()));
        assert!(!is_likely_urc("AT+CSQ", "+CSQ: 20,0"));
        assert!(is_likely_urc("AT+CSQ", "+CEREG: 1"));
        assert!(is_likely_urc("ATI", "+CEREG: 1"));
        assert!(!is_likely_urc("ATI", "ML307R-DL"));
    }
}
