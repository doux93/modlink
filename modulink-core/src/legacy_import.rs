//! 老工具 UART_V2.1 配置文件（Qt QSettings INI 方言）导入/导出。
//!
//! 格式要点（基于真实文件逆向）：
//! - 值中非 ASCII 字符写作 `\xHHHH`（UTF-16 码元）；含特殊字符的值用 `"..."` 包裹，
//!   内部引号写作 `\"`，反斜杠写作 `\\`；
//! - `[TabName]` 节以 `0=`、`1=` 编号存放分组名（可能缺失）；
//! - 指令节 `[1]..[N]` 全局连续编号，每个 Tab 固定 25 个按钮槽位；
//! - `[globalSetting]` 存放波特率、循环次数等；
//! - 脏数据常见：重复节号（取后者）、`delay=` 空值、`hex` 键时有时无。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 一条快捷指令（新格式）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuickCommand {
    pub text: String,
    /// "text" | "hex"
    pub format: String,
    pub append_newline: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<u32>,
    pub selected: bool,
    #[serde(default)]
    pub note: String,
    /// 在老工具中的槽位号（1 起），保留以便导出回老格式
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy_slot: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickCommandGroup {
    pub name: String,
    pub items: Vec<QuickCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopSettings {
    pub times: u32,
    pub interval_ms: u32,
}

/// 导入结果 = 新格式 quickcmds 文件 + 导入告警。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickCmdsFile {
    pub schema: String,
    pub name: String,
    pub groups: Vec<QuickCommandGroup>,
    pub r#loop: LoopSettings,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// 每个 Tab 固定 25 个按钮槽位（老工具 UI 布局决定）。
const SLOTS_PER_TAB: u32 = 25;

// ---------------------------------------------------------------- 解码

/// 解码 QSettings 的值：去引号、处理 `\"` `\\` `\xHHHH` 转义。
fn decode_value(raw: &str) -> String {
    let raw = raw.trim();
    let inner = if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        &raw[1..raw.len() - 1]
    } else {
        raw
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    // QSettings 将非 ASCII 存为 UTF-16 码元序列，需要处理代理对
    let mut pending_units: Vec<u16> = Vec::new();
    let flush_units = |units: &mut Vec<u16>, out: &mut String| {
        if !units.is_empty() {
            out.push_str(&String::from_utf16_lossy(units));
            units.clear();
        }
    };
    while let Some(c) = chars.next() {
        if c != '\\' {
            flush_units(&mut pending_units, &mut out);
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('x') => {
                chars.next();
                let mut hex = String::new();
                while hex.len() < 4 {
                    match chars.peek() {
                        Some(h) if h.is_ascii_hexdigit() => {
                            hex.push(*h);
                            chars.next();
                        }
                        _ => break,
                    }
                }
                if let Ok(unit) = u16::from_str_radix(&hex, 16) {
                    pending_units.push(unit);
                } else {
                    flush_units(&mut pending_units, &mut out);
                    out.push_str("\\x");
                    out.push_str(&hex);
                }
            }
            Some('"') => {
                flush_units(&mut pending_units, &mut out);
                out.push('"');
                chars.next();
            }
            Some('\\') => {
                flush_units(&mut pending_units, &mut out);
                out.push('\\');
                chars.next();
            }
            Some('n') => {
                flush_units(&mut pending_units, &mut out);
                out.push('\n');
                chars.next();
            }
            Some('r') => {
                flush_units(&mut pending_units, &mut out);
                out.push('\r');
                chars.next();
            }
            Some('t') => {
                flush_units(&mut pending_units, &mut out);
                out.push('\t');
                chars.next();
            }
            _ => {
                flush_units(&mut pending_units, &mut out);
                out.push('\\');
            }
        }
    }
    flush_units(&mut pending_units, &mut out);
    out
}

fn parse_bool(v: &str) -> bool {
    v.trim().eq_ignore_ascii_case("true")
}

/// 解析后的原始节数据。
#[derive(Default, Debug, Clone)]
struct RawSection {
    entries: BTreeMap<String, String>,
}

/// 把 ini 文本切成节（重复节号时后者覆盖前者的同名键）。
fn parse_sections(content: &str) -> BTreeMap<String, RawSection> {
    let mut sections: BTreeMap<String, RawSection> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current = Some(line[1..line.len() - 1].trim().to_string());
            sections.entry(current.clone().unwrap()).or_default();
            continue;
        }
        if let (Some(sec), Some(eq)) = (&current, line.find('=')) {
            let key = line[..eq].trim().to_string();
            let val = line[eq + 1..].to_string();
            sections.get_mut(sec).unwrap().entries.insert(key, val); // 重复键取后者
        }
    }
    sections
}

/// 导入老工具 ini，`name` 为默认配置名（一般取文件名）。
pub fn import_legacy_ini(content: &str, name: &str) -> QuickCmdsFile {
    let sections = parse_sections(content);
    let mut warnings: Vec<String> = Vec::new();

    // 1. 全局设置
    let mut lp = LoopSettings {
        times: 1,
        interval_ms: 300,
    };
    if let Some(gs) = sections.get("globalSetting") {
        if let Some(v) = gs.entries.get("runTimes") {
            if let Ok(n) = v.trim().parse::<u32>() {
                lp.times = n;
            }
        }
        if let Some(v) = gs.entries.get("delay") {
            if let Ok(n) = v.trim().parse::<u32>() {
                lp.interval_ms = n;
            }
        }
    }

    // 2. Tab 名（可能缺失）
    let mut tab_names: BTreeMap<u32, String> = BTreeMap::new();
    if let Some(tn) = sections.get("TabName") {
        for (k, v) in &tn.entries {
            if let Ok(idx) = k.trim().parse::<u32>() {
                tab_names.insert(idx, decode_value(v));
            }
        }
    }

    // 3. 数字节 → 指令槽位
    let mut slots: BTreeMap<u32, QuickCommand> = BTreeMap::new();
    let mut max_slot = 0u32;
    for (sec_name, sec) in &sections {
        let Ok(n) = sec_name.parse::<u32>() else {
            continue;
        };
        if n == 0 {
            warnings.push("忽略非法节号 [0]".into());
            continue;
        }
        max_slot = max_slot.max(n);
        let cmd_raw = sec
            .entries
            .get("cmd")
            .map(|v| decode_value(v))
            .unwrap_or_default();
        if cmd_raw.trim().is_empty() {
            continue; // 空槽
        }
        let is_hex = sec
            .entries
            .get("hex")
            .map(|v| parse_bool(v))
            .unwrap_or(false);
        if is_hex {
            let cleaned: String = cmd_raw.chars().filter(|c| !c.is_whitespace()).collect();
            if cleaned.chars().any(|c| !c.is_ascii_hexdigit()) || cleaned.len() % 2 != 0 {
                warnings.push(format!(
                    "槽位 {n}: hex 数据含非法字符或奇数长度，已按原样导入"
                ));
            }
        }
        let delay_ms = sec
            .entries
            .get("delay")
            .and_then(|v| v.trim().parse::<u32>().ok());
        slots.insert(
            n,
            QuickCommand {
                text: cmd_raw,
                format: if is_hex { "hex".into() } else { "text".into() },
                append_newline: sec
                    .entries
                    .get("enter")
                    .map(|v| parse_bool(v))
                    .unwrap_or(true),
                delay_ms,
                selected: sec
                    .entries
                    .get("select")
                    .map(|v| parse_bool(v))
                    .unwrap_or(false),
                note: String::new(),
                legacy_slot: Some(n),
            },
        );
    }

    // 4. 按每 Tab 25 槽切分分组
    let tab_count = if tab_names.is_empty() {
        if max_slot == 0 {
            0
        } else {
            (max_slot - 1) / SLOTS_PER_TAB + 1
        }
    } else {
        let by_name = tab_names.keys().max().map(|m| m + 1).unwrap_or(0);
        let by_slot = if max_slot == 0 {
            0
        } else {
            (max_slot - 1) / SLOTS_PER_TAB + 1
        };
        by_name.max(by_slot)
    };

    let mut groups = Vec::new();
    for t in 0..tab_count {
        let lo = t * SLOTS_PER_TAB + 1;
        let hi = lo + SLOTS_PER_TAB - 1;
        let items: Vec<QuickCommand> = (lo..=hi).filter_map(|i| slots.get(&i).cloned()).collect();
        let group_name = tab_names
            .get(&t)
            .cloned()
            .unwrap_or_else(|| format!("Tab {}", t + 1));
        // TabName 缺失且只有一个 Tab 时用文件名作组名
        let group_name = if tab_names.is_empty() && tab_count == 1 {
            name.to_string()
        } else {
            group_name
        };
        if items.is_empty() && tab_names.get(&t).is_none() {
            continue; // 无名且全空的 Tab 不生成分组
        }
        groups.push(QuickCommandGroup {
            name: group_name,
            items,
        });
    }

    QuickCmdsFile {
        schema: "modulink/quickcmds@1".into(),
        name: name.to_string(),
        groups,
        r#loop: lp,
        warnings,
    }
}

// ---------------------------------------------------------------- 导出

/// 编码为 QSettings 兼容的值。
fn encode_value(s: &str) -> String {
    let needs_quote =
        s.contains(|c: char| c == '"' || c == ',' || c == ';' || c == '=' || c == ' ');
    let mut body = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => body.push_str("\\\""),
            '\\' => body.push_str("\\\\"),
            '\n' => body.push_str("\\n"),
            '\r' => body.push_str("\\r"),
            '\t' => body.push_str("\\t"),
            c if (c as u32) < 0x80 => body.push(c),
            c => {
                let mut units = [0u16; 2];
                for u in c.encode_utf16(&mut units) {
                    body.push_str(&format!("\\x{:x}", u));
                }
            }
        }
    }
    if needs_quote {
        format!("\"{}\"", body)
    } else {
        body
    }
}

/// 导出回老工具 ini 格式（给未迁移的同事/客户）。
pub fn export_legacy_ini(file: &QuickCmdsFile) -> String {
    let mut out = String::new();
    out.push_str("[globalSetting]\n");
    out.push_str(&format!("runTimes={}\n", file.r#loop.times));
    out.push_str(&format!("delay={}\n", file.r#loop.interval_ms));
    out.push_str("baudrate=115200\ndatabits=8\n\n");

    out.push_str("[TabName]\n");
    for (i, g) in file.groups.iter().enumerate() {
        out.push_str(&format!("{}={}\n", i, encode_value(&g.name)));
    }
    out.push('\n');

    for (gi, g) in file.groups.iter().enumerate() {
        let base = gi as u32 * SLOTS_PER_TAB;
        let mut used: Vec<u32> = Vec::new();
        // 优先按原槽位放回，冲突/缺失则顺序填空位
        for item in &g.items {
            let slot_in_tab = item
                .legacy_slot
                .map(|s| (s - 1) % SLOTS_PER_TAB + 1)
                .filter(|s| !used.contains(s))
                .unwrap_or_else(|| {
                    (1..=SLOTS_PER_TAB)
                        .find(|s| !used.contains(s))
                        .unwrap_or(SLOTS_PER_TAB)
                });
            used.push(slot_in_tab);
            let n = base + slot_in_tab;
            out.push_str(&format!("[{}]\n", n));
            out.push_str(&format!("cmd={}\n", encode_value(&item.text)));
            out.push_str(&format!("enter={}\n", item.append_newline));
            out.push_str(&format!("select={}\n", item.selected));
            out.push_str(&format!(
                "delay={}\n",
                item.delay_ms.map(|d| d.to_string()).unwrap_or_default()
            ));
            if item.format == "hex" {
                out.push_str("hex=true\n");
            }
            out.push('\n');
        }
    }
    out
}

// ---------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn fixture(name: &str) -> String {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures")
            .join(name);
        fs::read_to_string(p).expect("read fixture")
    }

    #[test]
    fn decode_plain() {
        assert_eq!(decode_value("AT+CSQ"), "AT+CSQ");
    }

    #[test]
    fn decode_quoted_with_escaped_quotes() {
        assert_eq!(
            decode_value(r#""AT+MIPOPEN=0,\"UDP\",\"8.137.154.246\",2046""#),
            r#"AT+MIPOPEN=0,"UDP","8.137.154.246",2046"#
        );
    }

    #[test]
    fn decode_unicode_escapes() {
        // \x6d4b\x8bd5 = 测试
        assert_eq!(decode_value(r"UDP\x6d4b\x8bd5"), "UDP测试");
        // 全角与代理对无关的混合
        assert_eq!(
            decode_value(r#""AT+MTTSPLAY=\"\x4f60\x597d\xff01\"""#),
            "AT+MTTSPLAY=\"你好！\""
        );
    }

    #[test]
    fn roundtrip_value_encoding() {
        for s in [
            r#"AT+MIPOPEN=0,"TCP","8.137.154.246",2046"#,
            "UDP测试",
            "AT+CSQ",
            "含 空格 与,逗号",
        ] {
            assert_eq!(decode_value(&encode_value(s)), s, "roundtrip failed: {s}");
        }
    }

    #[test]
    fn import_config_ini_full() {
        let f = import_legacy_ini(&fixture("config.ini"), "config");
        // [TabName] 有 0=Cat1, 1=NB-IoT, 3=page3, 5=page5 → 至少 6 组（150 槽）
        assert!(f.groups.len() >= 6, "groups: {}", f.groups.len());
        assert_eq!(f.groups[0].name, "Cat1");
        assert_eq!(f.groups[1].name, "NB-IoT");
        // 槽 1: AT+CGATT? delay=300 select=true
        let first = &f.groups[0].items[0];
        assert_eq!(first.text, "AT+CGATT?");
        assert_eq!(first.delay_ms, Some(300));
        assert!(first.selected);
        // 槽 4 是 hex 帧
        let hex_item = f.groups[0]
            .items
            .iter()
            .find(|i| i.legacy_slot == Some(4))
            .expect("slot 4 exists");
        assert_eq!(hex_item.format, "hex");
        // 全局循环设置
        assert_eq!(f.r#loop.times, 1000);
        assert_eq!(f.r#loop.interval_ms, 300);
    }

    #[test]
    fn import_m5311_no_tabname() {
        let f = import_legacy_ini(&fixture("m5311.ini"), "m5311");
        // 无 TabName → 组名取文件名（槽位共 50 → 2 个 Tab，但第二个 Tab 有内容）
        assert!(!f.groups.is_empty());
        let all: Vec<&QuickCommand> = f.groups.iter().flat_map(|g| &g.items).collect();
        assert!(all.iter().any(|c| c.text == "AT+CSQ"));
        // 中文指令标签正确解码: UDP测试
        assert!(
            all.iter().any(|c| c.text == "UDP测试"),
            "unicode decode in real file"
        );
    }

    #[test]
    fn import_skips_empty_slots() {
        let f = import_legacy_ini(&fixture("m5311.ini"), "m5311");
        for g in &f.groups {
            for c in &g.items {
                assert!(!c.text.trim().is_empty());
            }
        }
    }

    #[test]
    fn export_then_import_roundtrip() {
        let f = import_legacy_ini(&fixture("config.ini"), "config");
        let ini = export_legacy_ini(&f);
        let f2 = import_legacy_ini(&ini, "config");
        let items = |x: &QuickCmdsFile| -> Vec<(String, String, bool)> {
            x.groups
                .iter()
                .flat_map(|g| {
                    g.items
                        .iter()
                        .map(|c| (c.text.clone(), c.format.clone(), c.append_newline))
                })
                .collect()
        };
        assert_eq!(items(&f), items(&f2));
    }

    #[test]
    fn import_other_fixtures_no_panic() {
        for name in [
            "M6312.ini",
            "NB模组测试.ini",
            "cmiot_config_2025-09-28#14-42-39.ini",
        ] {
            let f = import_legacy_ini(&fixture(name), name);
            assert!(
                f.groups.iter().map(|g| g.items.len()).sum::<usize>() > 0,
                "{name} imported zero commands"
            );
        }
    }
}
