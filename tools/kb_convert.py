#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
知识库转换器：anythingllm_kb/at_commands → data/at_docs/*.yaml

消费用户已有的结构化知识库（DITA 提取产物）:
  - commands_index.jsonl: 每行一条指令的元数据(protocol/command/title/shortdesc/syntax)
  - <protocol>/<CMD>.md:  含 参数描述 / 示例 章节的正文

产出工具本地知识文件（KnowledgeProvider ② 本地快照层）:
  - data/at_docs/<protocol>.yaml

用法:
  python tools/kb_convert.py --kb E:/documents/anythingllm_kb/at_commands --out data/at_docs
"""
import argparse
import datetime
import json
import os
import re
import shutil
import sys
import tempfile
from collections import defaultdict
from pathlib import Path, PurePosixPath
from typing import Optional
import hashlib

MAX_DOC_LEN = 1500


def write_text_lf(path: Path, content: str, encoding: str = "utf-8") -> None:
    """Atomically replace a generated text file using LF line endings."""
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding=encoding, newline="\n") as destination:
            destination.write(content)
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def path_is_within(path: Path, parent: Path) -> bool:
    """Return whether *path* is *parent* or one of its descendants."""
    try:
        path.relative_to(parent)
        return True
    except ValueError:
        return False


def validate_manual_source_root(root: Path) -> None:
    """Reject generated knowledge packs as compiler input.

    A delivered pack deliberately mirrors ``<repo>/topics/*.md`` and therefore
    looks like a source tree to the harvester.  Compiling it again loses maps that
    were excluded by the first build and can overwrite its own contract files.
    """
    if not root.is_dir():
        raise ValueError(f"手册源目录不存在: {root}")
    contract_files = [
        name
        for name in ("manifest.json", "topic_paths.jsonl")
        if (root / name).exists()
    ]
    if contract_files:
        raise ValueError(
            "--root 指向的是已生成的知识包（含 "
            + "、".join(contract_files)
            + "），请改用原始 manuals 源目录"
        )



def topic_id_for(rel_path: str, title: str) -> str:
    """Generate a stable 16-char hex topic ID from the relative path."""
    return hashlib.sha1(rel_path.encode("utf-8")).hexdigest()[:16]

def sanitize_preview(preview: str) -> str:
    """移除 preview 中的内部路径、来源标记、仓库标识和元数据块，保留可对外交付的摘要内容。"""
    s = preview
    s = re.sub(r"仓库：[^\n-]*", "", s)
    s = re.sub(r"来源：[^\n-]*", "", s)
    s = re.sub(r"developmentdomain/\S*", "", s)
    s = s.replace("anythingllm_kb", "")
    # 移除内部文档模板元语句
    s = re.sub(r"研发中心产品手册发布项目模板", "", s)
    s = re.sub(r"仅供文档工程师使用", "", s)
    # 移除残留的元数据片段（手册+类型块、仓库+反引号标识）
    s = re.sub(r"手册：[^`]*`[a-z0-9_]+`类型：[^一-鿿`]*`?", "", s)
    s = re.sub(r"手册：[^一-鿿]{2,40}类型：[^一-鿿]{2,30}`?", "", s)
    s = re.sub(r"仓库：[^一-鿿`]*`[a-z0-9_]+`", "", s)
    # 移除独立的反引号仓库标识（如 `5glanfunctionusageguide`）
    s = re.sub(r"`[a-z0-9][a-z0-9_]{3,}`", "", s)
    # 移除残留的 .dita / .md 文件名引用
    s = re.sub(r"`?\w+\.(dita|md)`?", "", s)
    # 移除残留的"类型：xxx"元数据
    s = re.sub(r"类型：[^一-鿿`]{0,30}`?", "", s)
    # 移除残留的"手册：xxx"元数据头
    s = re.sub(r"手册：[^一-鿿`]{2,40}", "", s)
    # 清理孤立反引号
    s = s.replace("`", "")
    s = re.sub(r"-\s+", "", s)
    s = re.sub(r"\s{3,}", " ", s)
    return s.strip()



MODEL_CODE_RE = re.compile(
    r"(?<![A-Z0-9])((?:ML|MN|MR|MF|MS)\s*\d{3,4}[A-Z]?(?:-[A-Z0-9]+)*)(?![A-Z0-9])",
    re.I,
)


def infer_model_codes(*texts: str) -> list[str]:
    """Conservatively extract model-like product codes from source metadata."""
    models = {
        match.upper().replace(" ", "")
        for text in texts
        for match in MODEL_CODE_RE.findall(
            re.sub(
                r"(?<=[A-Z0-9])(?=(?:ML|MN|MR|MF|MS)\s*\d)",
                " ",
                (text or "").upper(),
            )
        )
    }
    return sorted(models)


def infer_topic_variants(relative_path: str) -> list[str]:
    """Preserve product branches that a generic manual title cannot express."""
    normalized = str(PurePosixPath(relative_path)).lower()
    return ["nb-iot"] if "nb_topics" in normalized or "common_basic_topics" in normalized else []


def normalize_model_code(model: str) -> str:
    return model.strip().upper()


def model_series(model: str) -> str:
    """Return the catalog series key, e.g. ML307C -> ML307 and MN328-BX -> MN328."""
    match = re.match(r"^([A-Z]{2}\d+)", normalize_model_code(model))
    return match.group(1) if match else ""


def topic_scope_kind(
    topic_models: list[str],
    selected_models: set[str],
    include_series: bool,
    include_generic: bool,
) -> Optional[str]:
    normalized = {normalize_model_code(model) for model in topic_models if model.strip()}
    if not normalized:
        return "generic" if include_generic else None
    if normalized & selected_models:
        return "exact"
    if include_series:
        selected_series = {model_series(model) for model in selected_models}
        topic_series = {model_series(model) for model in normalized}
        if (selected_series - {""}) & (topic_series - {""}):
            return "series"
    return None


def resolved_topic_scope(
    topic: dict,
    selected_models: set[str],
    include_series: bool,
    include_unknown: bool,
) -> Optional[str]:
    """Resolve a canonical topic through any compatible owner without flattening provenance."""
    owners = topic.get("owners", [])
    if not owners:
        legacy = topic_scope_kind(
            topic.get("models", []), selected_models, include_series, include_unknown
        )
        return "unknown" if legacy == "generic" else legacy
    owner_scopes = []
    has_unknown = False
    for owner in owners:
        if owner.get("scope_source") == "unknown" or not owner.get("models"):
            has_unknown = True
            continue
        scope = topic_scope_kind(owner.get("models", []), selected_models, include_series, False)
        if scope:
            owner_scopes.append(scope)
    if "exact" in owner_scopes:
        return "exact"
    if "series" in owner_scopes:
        return "series"
    if include_unknown and has_unknown:
        return "unknown"
    return None


def extract_section(md: str, title: str) -> str:
    """提取 '## title' 到下一个 '## ' 之间的正文。"""
    m = re.search(rf"^## {re.escape(title)}\s*$(.*?)(?=^## |\Z)", md, re.M | re.S)
    return m.group(1).strip() if m else ""


def clean_text(s: str) -> str:
    s = re.sub(r"```text\n?|```\n?", "", s)          # 去代码围栏
    s = re.sub(r"\n{3,}", "\n\n", s)
    return s.strip()


def pick_syntax(syntax_list):
    """从 syntax 数组挑最有代表性的设置命令行。"""
    for s in syntax_list:
        if s.startswith("AT") and "=" in s and "<" in s and not s.endswith("=?"):
            return s
    return syntax_list[0] if syntax_list else ""


def extract_examples(md: str, limit=2):
    """示例章节里的代码块，取前 limit 个。"""
    sec = extract_section(md, "示例")
    blocks = re.findall(r"```text\n(.*?)```", sec, re.S)
    return [b.strip() for b in blocks[:limit]]


RESTRICTION_WORDS = re.compile(
    r"(?:暂不支持|不支持|只支持|仅支持|does\s+not\s+support|not\s+supported|only\s+supports?)",
    re.I,
)


def restriction_lines(md: str) -> list[str]:
    """Return explicit model-scoped support restrictions from a topic body."""
    lines = []
    for raw in md.splitlines():
        line = raw.strip().lstrip("> ")
        if RESTRICTION_WORDS.search(line) and infer_model_codes(line):
            lines.append(line)
    return lines


def extract_unsupported_command_models(md: str) -> list[str]:
    """Extract models explicitly stated as not supporting the whole command."""
    models = set()
    for line in restriction_lines(md):
        if re.search(r"(?:暂不支持|不支持).{0,8}(?:该|此)?命令|(?:command).{0,12}(?:not\s+supported|does\s+not\s+support)", line, re.I):
            models.update(infer_model_codes(line))
    return sorted(models)


def extract_parameter_constraints(md: str, default_models: Optional[list[str]] = None) -> list[dict]:
    """Compile explicit model notes such as 'ML307C: only supports passive mode'.

    Keep polarity explicit: "only supports" creates an allow-list while
    "does not support" creates a deny-list.  A missing/default argument restriction
    is represented as ``required`` rather than as an invented value.
    """
    section = extract_section(md, "参数描述")
    if not section:
        return []
    starts = list(re.finditer(r"^`<([A-Za-z0-9_]+)>`", section, re.M))
    constraints = []
    seen = set()
    for index, start in enumerate(starts):
        parameter = start.group(1)
        block = section[start.start() : starts[index + 1].start() if index + 1 < len(starts) else len(section)]
        values = {
            value: label.strip().rstrip("。.")
            for value, label in re.findall(r"^-\s*`([^`]+)`\s*:\s*(.+)$", block, re.M)
        }
        if not values:
            continue
        position = None
        for syntax_line in re.findall(r"^AT[^\n]+", md, re.M | re.I):
            if f"<{parameter}>" not in syntax_line or "=" not in syntax_line:
                continue
            syntax_arguments = syntax_line.split("=", 1)[1]
            tokens = [token.strip(" []") for token in syntax_arguments.split(",")]
            try:
                position = tokens.index(f"<{parameter}>")
                break
            except ValueError:
                continue
        for line in block.splitlines():
            if not RESTRICTION_WORDS.search(line):
                continue
            enum_match = re.match(r"^-\s*`([^`]+)`\s*:", line)
            enum_value = enum_match.group(1) if enum_match else None
            # DITA often emits several model-specific notes on one Markdown line.
            # Parse each clause independently so one model's polarity cannot leak
            # into the neighbouring clause.
            clauses = re.split(r"）\s*（注：", line)
            for clause in clauses:
                if not RESTRICTION_WORDS.search(clause):
                    continue
                models = infer_model_codes(clause)
                attribution = "clause"
                if not models and default_models and not re.search(
                    r"\b(?:ML|MN|MR|MF|MS)\s*\d{3,4}", clause, re.I
                ):
                    models = list(default_models)
                    attribution = "owner_default"
                if not models:
                    continue
                normalized_clause = re.sub(r"[\s_-]+", "", clause).lower()
                mentioned_values = [
                    value
                    for value, label in values.items()
                    if re.sub(r"[\s_-]+", "", label).lower() in normalized_clause
                ]
                if enum_value and enum_value not in mentioned_values:
                    mentioned_values.append(enum_value)
                is_required = bool(re.search(
                    r"不支持(?:缺省|默认)|必须配置|cannot\s+be\s+omitted", clause, re.I
                ))
                is_denied = bool(re.search(
                    r"不支持|暂不支持|not\s+supported|does\s+not\s+support", clause, re.I
                ))
                is_allowed = bool(re.search(r"只支持|仅支持|only\s+supports?", clause, re.I))
                if not (is_required or mentioned_values):
                    continue
                selector_match = re.search(
                    rf'"([A-Za-z0-9_]+)"[^\n]*<{re.escape(parameter)}>', md, re.I
                )
                selector = selector_match.group(1) if selector_match else parameter
                if not selector_match:
                    for suffix in ("_mode", "_type", "_enable"):
                        if selector.endswith(suffix):
                            selector = selector[: -len(suffix)]
                            break
                if is_required:
                    kind = "required"
                    constraint_values = []
                elif is_denied and not is_allowed:
                    kind = "denied"
                    constraint_values = mentioned_values
                elif is_allowed:
                    kind = "allowed"
                    constraint_values = mentioned_values
                else:
                    continue
                if kind != "required" and not constraint_values:
                    confidence = "review_required"
                    enforce = False
                elif attribution == "owner_default":
                    # The clause named no model, so the note was spread over every model
                    # the owning DITAMAP declares — for a shared reference guide that can
                    # be 40+ models.  Keep it as displayable evidence, never as a gate.
                    confidence = "inferred"
                    enforce = False
                else:
                    confidence = "parsed"
                    enforce = True
                note = clause.strip()
                key = (parameter, tuple(models), kind, tuple(constraint_values), note)
                if key in seen:
                    continue
                seen.add(key)
                constraints.append({
                    "parameter": parameter,
                    "selector": selector,
                    "position": position,
                    "models": models,
                    "kind": kind,
                    "values": constraint_values,
                    "allowed_values": constraint_values if kind == "allowed" else [],
                    "note": note,
                    "attribution": attribution,
                    "confidence": confidence,
                    "enforce": enforce,
                })
    return constraints


def convert(kb_dir: Path, out_dir: Path) -> dict:
    index_file = kb_dir / "commands_index.jsonl"
    if not index_file.exists():
        sys.exit(f"未找到 {index_file}")

    by_protocol = defaultdict(list)
    protocol_models = defaultdict(set)
    stats = {"total": 0, "no_md": 0}

    for line in index_file.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        meta = json.loads(line)
        cmd = meta["command"]
        protocol = meta.get("protocol", "misc").lower()

        # 找对应 md 正文（output_file 路径相对仓库根，取文件名在 kb_dir 下按协议目录找）
        md_name = Path(meta.get("output_file", "")).name or f"{cmd}.md"
        md_path = kb_dir / protocol / md_name
        md = md_path.read_text(encoding="utf-8") if md_path.exists() else ""
        if not md:
            stats["no_md"] += 1

        params = clean_text(extract_section(md, "参数描述")) if md else ""
        doc_parts = [meta.get("shortdesc", "").strip()]
        if params:
            doc_parts.append("参数：\n" + params)
        doc = "\n\n".join(p for p in doc_parts if p)
        if len(doc) > MAX_DOC_LEN:
            doc = doc[:MAX_DOC_LEN] + "…(截断，完整内容见知识库)"

        is_urc = meta.get("kind", "").lower() != "at command" or cmd.startswith("+")
        models = infer_model_codes(md, meta.get("title", ""), meta.get("shortdesc", ""))
        protocol_models[protocol].update(models)
        source = meta.get("source_file", "")
        by_protocol[protocol].append(
            {
                "cmd": cmd,
                "syntax": pick_syntax(meta.get("syntax", [])),
                "category": protocol,
                "kind": "urc" if is_urc else "command",
                "risk": "none",
                "title": meta.get("title", ""),
                "doc": doc,
                "examples": extract_examples(md) if md else [],
                "source": source,
                "models": models,
                "sources": [source] if source else [],
            }
        )
        stats["total"] += 1

    out_dir.mkdir(parents=True, exist_ok=True)
    try:
        import yaml

        def dump(obj, f):
            yaml.safe_dump(obj, f, allow_unicode=True, sort_keys=False, width=100)
    except ImportError:
        # YAML 是 JSON 超集，无 pyyaml 时用 JSON 落盘同样可被 serde_yaml 解析
        def dump(obj, f):
            json.dump(obj, f, ensure_ascii=False, indent=2)

    for protocol, commands in sorted(by_protocol.items()):
        # Functional manuals describe one protocol-wide command set. Individual topics often
        # mention only model-specific exceptions, so use the union instead of under-scoping.
        models = sorted(protocol_models[protocol])
        for command in commands:
            command["models"] = list(models)
        out = {
            "schema": "modulink/at_docs@1",
            "protocol": protocol.upper(),
            "applies_to": list(models),
            "source": "anythingllm_kb/at_commands (DITA 手册自动提取)",
            "commands": commands,
        }
        out_path = out_dir / f"{protocol}.yaml"
        with open(out_path, "w", encoding="utf-8", newline="\n") as f:
            dump(out, f)
        print(f"  {out_path}  ({len(commands)} 条)")

    return stats


# ---------------------------------------------------------------- 典型用例转换

AT_LINE = re.compile(r"^(AT[+*&A-Z][^\s/]*(?:[^/\n]|/(?!/))*?)\s*(?://(.*))?$", re.I)
URC_LINE = re.compile(r"^(\+[A-Z][A-Z0-9]*)\b(:?[^/\n]*)\s*(?://(.*))?$")
FINAL_LINE = re.compile(r"^(OK|ERROR|\+CME ERROR:.*|\+CMS ERROR:.*)\s*(?://.*)?$")
MODEL_CODE = re.compile(r"\b(M[LNRSF]\d{3}[A-Z0-9-]*|M5310[A-Z0-9-]*|M5311[A-Z0-9-]*|M6312|MR880[A-Z0-9-]*)\b", re.I)

CATEGORY_RULES = [
    (re.compile(r"TCP|UDP|SOCKET|IP", re.I), "socket"),
    (re.compile(r"MQTT|加密接入|非加密接入|缓存模式", re.I), "mqtt"),
    (re.compile(r"HTTP|GET请求|POST数据|API-|设备资源|LWM2M", re.I), "http"),
    (re.compile(r"FTP", re.I), "ftp"),
    (re.compile(r"驻网|注网|网络|BAND|频点|IPv6|拨号|5G\s*LAN|5G\s*切片|切片|透传", re.I), "network"),
    (re.compile(r"睡眠|唤醒|休眠|功耗|PSM|省电|深睡|断电", re.I), "power"),
    (re.compile(r"短信|PDU|SMS", re.I), "sms"),
    (re.compile(r"GPIO|ADC|LED|串口|波特率|看门狗|VBAT|温度|保温|温保|SIM卡|热插拔|授时", re.I), "hardware"),
    (re.compile(r"FOTA|升级", re.I), "fota"),
]


def categorize(title: str) -> str:
    for pattern, cat in CATEGORY_RULES:
        if pattern.search(title):
            return cat
    return "misc"


_MUTATING_PREFIXES = [
    "AT+MREBOOT", "AT+CFUN=0", "AT+CFUN=1,1", "AT+MFLASH", "AT+WFLASH",
    "AT+MRESTORE", "AT+MRESET", "AT+RESET",
]
_CONNECT_PREFIXES = [
    "AT+MIPOPEN", "AT+MIPLISTEN", "AT+MIPSEND", "AT+MQTTCONNECT", "AT+MQTTSUB",
    "AT+MQTTPUB", "AT+MFTPCONN", "AT+MFTPSTOR", "AT+MFTPRETR", "AT+MFTPDEL",
    "AT+MHTTPURL", "AT+MHTTPSEND", "AT+HTTPURL", "AT+HTTPACTION",
    "AT+NSOCR", "AT+NSOST", "AT+CIPSTART", "AT+CIPSEND",
]


def infer_execution_mode(steps):
    """根据步骤指令推断执行影响范围（与 modulink-core::infer_execution_mode 一致）。
    含写 Flash/重启/关机/建立连接/赋值写入 → mutating；纯查询 → readonly；否则 unknown。"""
    has_readonly = False
    for step in steps:
        for raw in step.get("send", []):
            upper = raw.strip().upper()
            if not upper:
                continue
            if any(upper.startswith(p) for p in _MUTATING_PREFIXES):
                return "mutating"
            if (upper.startswith("AT+CGDCONT=") or upper.startswith("AT+CGACT=")) and not upper.endswith("?"):
                return "mutating"
            if any(upper.startswith(p) for p in _CONNECT_PREFIXES):
                return "mutating"
            if "=" in upper and not upper.endswith("?") and not upper.endswith("=?"):
                return "mutating"
            if upper.endswith("?") or upper.startswith(("AT+CSQ", "AT+CEREG", "AT+CGPADDR", "ATI", "AT+CGMM", "AT+CGMR")):
                has_readonly = True
    return "readonly" if has_readonly else "unknown"


def slugify(text: str) -> str:
    out = re.sub(r"[^a-z0-9]+", "_", text.lower()).strip("_")
    return out[:40] or "case"


def urc_to_pattern(prefix: str) -> str:
    """'+MIPOPEN' → '^\\+MIPOPEN:' 宽松匹配（参数因型号/场景而异）"""
    return r"^\{}\b".format(prefix.replace("+", "+").replace("+", r"\+", 1)) + ""


def parse_example_block(block: str):
    """把手册示例代码块解析成步骤列表: [{cmd, note, urcs:[prefix...]}]"""
    steps = []
    for rawline in block.splitlines():
        line = rawline.strip()
        if not line:
            continue
        m = AT_LINE.match(line)
        if m and line.upper().startswith("AT"):
            steps.append({
                "cmd": m.group(1).strip().rstrip(","),
                "note": (m.group(2) or "").strip(),
                "urcs": [],
            })
            continue
        if FINAL_LINE.match(line):
            continue
        m = URC_LINE.match(line)
        if m and steps:
            steps[-1]["urcs"].append(m.group(1))
    # 过滤：无指令的块
    return [s for s in steps if s["cmd"]]


USECASE_MANUAL_DIRS = [
    "communicationprocessexamples",
    "mqttusermanual",
    "http_httpsusermanual",
    "ftpusermanual",
]


def convert_usecases(root: Path, out_dir: Path):
    """
    扫描通信流程示例 + MQTT/HTTP/FTP 用户手册的 topics，
    每个含 AT 序列代码块的章节生成一个 usecase 流程（kind=usecase, verified=false）。
    跳过 localization 英文重复稿；按内容哈希去重，跨手册合并适用型号。
    单指令示例（如型号专属的 ADC/GPIO 演示）也保留为独立用例。
    MQTT/HTTP/FTP 手册的 manual_title 为通用名，型号从正文/文件名抽取；
    无法确定型号时 applies_to 留空（作为通用参考用例）。
    """
    import hashlib
    topics = []
    for sub in USECASE_MANUAL_DIRS:
        topics.extend(root.glob(f"{sub}/topics/*.md"))
        topics.extend(root.glob(f"{sub}/**/topics/*.md"))
    seen = set()
    deduped = []
    for p in sorted(topics):
        if "localization" in p.name:
            continue
        rp = p.resolve()
        if rp in seen:
            continue
        seen.add(rp)
        deduped.append(p)
    topics = deduped
    print(f"扫描 {len(topics)} 个中文用例主题（{len(USECASE_MANUAL_DIRS)} 本手册）…")

    # 指令参考页（标题即指令名，如 "AT+MFTPCONN 建立FTP连接" / "+MHTTPURC URC信息上报"）
    # 不生成用例：其示例是单指令演示，内容已在 at_docs 指令文档；只保留
    # 真正的示例/流程章节（"FTP示例 · 下载"这类连贯序列）。
    COMMAND_PAGE = re.compile(r"^\s*(AT[+*&]|\+[A-Z])")

    flows = {}
    for p in topics:
        text = p.read_text(encoding="utf-8", errors="replace")
        fm = parse_frontmatter(text)
        title = fm.get("title", p.stem)
        if COMMAND_PAGE.match(title):
            continue
        manual = fm.get("manual_title", "")
        body = FM_RE.sub("", text, count=1)
        models = sorted({m.upper() for m in MODEL_CODE.findall(manual + " " + p.name)})

        # 按 "## 小节" 切段，每段的 text 代码块合成一个序列
        sections = re.split(r"^## ", body, flags=re.M)
        candidates = []  # (sec_title, steps)
        for sec in sections:
            sec_title = sec.splitlines()[0].strip() if sec.strip() else ""
            if sec_title in ("快速索引", "") or "示例" == sec_title:
                sec_title = title
            blocks = re.findall(r"```text\n(.*?)```", sec, re.S)
            all_steps = []
            for b in blocks:
                all_steps.extend(parse_example_block(b))
            if all_steps:
                candidates.append((sec_title, all_steps))

        # 用例 = 连贯序列（每个功能模块保留完整流程）。单指令小节并入
        # "主题序列"（小节标题作步骤名）；合并后仍只有 1 步的主题整体丢弃——
        # "断开 FTP 连接"这类孤条没有流程价值，只稀释列表；其手册内容在
        # at_docs 指令文档里已可查，不需要重复成用例。
        multi = [(t, s) for t, s in candidates if len(s) >= 2]
        singles = [(t, s) for t, s in candidates if len(s) == 1]
        merged_sections = list(multi)
        if len(singles) >= 2:
            merged_steps = []
            for sec_title, steps_1 in singles:
                step = dict(steps_1[0])
                # 小节标题比指令注释更能说明这一步的意图
                if sec_title and sec_title != title and not step["note"]:
                    step["note"] = sec_title
                merged_steps.append(step)
            merged_sections.append((title, merged_steps))

        for sec_title, all_steps in merged_sections:
            if len(all_steps) < 2:
                continue  # 连贯用例门槛：至少 2 步
            digest = hashlib.md5(
                json.dumps([s["cmd"] for s in all_steps], ensure_ascii=False).encode()
            ).hexdigest()[:8]
            if digest in flows:
                # 同一序列出现在多本手册 → 合并适用型号
                flows[digest]["applies_to"] = sorted(
                    set(flows[digest]["applies_to"]) | {m.lower() for m in models}
                )
                continue

            steps = []
            for i, s in enumerate(all_steps):
                step = {
                    "id": f"s{i+1}",
                    "title": s["note"] or s["cmd"][:50],
                    "send": [s["cmd"]],
                    "timeout_ms": 8000,
                }
                rules = []
                if s["urcs"]:
                    step["expect_urc"] = {
                        "pattern": r"^\+" + re.escape(s["urcs"][0].lstrip("+")) + r"\b",
                        "timeout_ms": 15000,
                    }
                    rules.append({
                        "when": "final == 'OK' && urc == 1",
                        "result": "pass",
                    })
                    rules.append({
                        "default": True,
                        "result": "fail",
                        "reason": f"未收到预期上报 {s['urcs'][0]}（final={{final}}）",
                        "abort": True,
                    })
                else:
                    rules.append({"when": "final == 'OK'", "result": "pass"})
                    rules.append({
                        "default": True,
                        "result": "fail",
                        "reason": "返回 {final}",
                        "abort": True,
                    })
                step["evaluate"] = rules
                steps.append(step)

            display = f"{title}" if sec_title == title else f"{title} · {sec_title}"
            flows[digest] = {
                "schema": "modulink/diagflow@1",
                "id": f"uc_{slugify(categorize(display))}_{digest}",
                "name": display,
                "kind": "usecase",
                "category": categorize(display),
                "source": manual,
                "verified": False,
                "execution_mode": infer_execution_mode(steps),
                "applies_to": sorted({m.lower() for m in models}),
                "steps": steps,
            }

    out_dir.mkdir(parents=True, exist_ok=True)
    try:
        import yaml

        def load_yaml(path):
            return yaml.safe_load(path.read_text(encoding="utf-8"))

        def dump(obj, f):
            yaml.safe_dump(obj, f, allow_unicode=True, sort_keys=False, width=110)
    except ImportError:
        def load_yaml(path):
            return json.loads(path.read_text(encoding="utf-8"))

        def dump(obj, f):
            json.dump(obj, f, ensure_ascii=False, indent=2)

    # ---- FAE 复核成果保护 ----
    # 已 verified: true 的用例是人工复核资产（可能改过服务器地址/期待值等），
    # 重跑转换器时原样保留、跳过重建；仅重建未复核（verified: false）的文件。
    # 用例 id 尾部是"指令序列内容哈希"：手册示例变了 → 哈希变 → 生成新 id 文件，
    # 旧 verified 文件不会被自动覆盖，但会被报告出来提醒人工比对。
    preserved = {}
    for old in out_dir.glob("uc_*.yaml"):
        try:
            data = load_yaml(old)
        except Exception:
            print(f"  警告: {old.name} 解析失败，按未复核处理（将被重建）")
            old.unlink()
            continue
        if data and data.get("verified"):
            preserved[old.stem] = data
        else:
            old.unlink()

    from collections import Counter
    cats = Counter()
    written = 0
    for flow in flows.values():
        if flow["id"] in preserved:
            cats[preserved[flow["id"]].get("category", flow["category"])] += 1
            continue  # 同 id 已有人工复核版本，保留人工版
        with open(out_dir / f"{flow['id']}.yaml", "w", encoding="utf-8") as f:
            dump(flow, f)
        cats[flow["category"]] += 1
        written += 1

    # 报告孤儿：verified 文件的源示例已消失或内容已变（新哈希），提醒人工处理
    orphans = [stem for stem in preserved if stem not in flows]
    if preserved:
        print(f"保留人工复核用例 {len(preserved)} 个（跳过重建）")
    for stem in orphans:
        print(f"  提醒: {stem}.yaml 已复核，但源手册示例已变更或删除——"
              f"请人工比对（新版本若存在会以新 id 生成）")
    print(f"典型用例共 {len(flows)} 个（本次重建 {written}，保留复核 {len(preserved)}）→ {out_dir}")
    for cat, n in cats.most_common():
        print(f"  {cat}: {n}")


# ---------------------------------------------------------------- manuals 收割

FM_RE = re.compile(r"\A---\s*\n(.*?)\n---\s*\n", re.S)
FM_KV = re.compile(r'^(\w+):\s*"?(.*?)"?\s*$', re.M)
CMD_IN_TITLE = re.compile(r"((?:AT[+*][A-Z0-9]+)|(?:^\+[A-Z0-9]+))", re.I)


def parse_frontmatter(text: str) -> dict:
    m = FM_RE.match(text)
    if not m:
        return {}
    return dict(FM_KV.findall(m.group(1)))


APPLICABLE_TITLE_RE = re.compile(r"适用(?:模组|型号)|Applicable\s+(?:Model|Module)", re.I)
SOURCE_FRAGMENT_RE = re.compile(r"_([0-9a-f]{8})\.md$", re.I)


def map_id_for(frontmatter: dict) -> str:
    """DITAMAP names are reused across repositories; the pair is the publication identity."""
    repo = frontmatter.get("repo", "").strip()
    map_file = frontmatter.get("map_file", "").strip()
    return f"{repo}/{map_file}" if repo and map_file else ""


def topic_language(frontmatter: dict, body: str) -> str:
    map_file = frontmatter.get("map_file", "").lower()
    source_file = frontmatter.get("source_file", "").lower()
    if re.search(r"(?:^|[_-])en(?:[_\.-]|$)", map_file) or "/localization/" in source_file:
        return "en"
    return "zh" if re.search(r"[\u4e00-\u9fff]", body) else "en"


def normalized_topic_body(text: str) -> str:
    """Remove converter-injected provenance while retaining the rendered semantic body."""
    body = FM_RE.sub("", text, count=1)
    body = re.sub(r"^[-\s]*(手册|仓库|来源|类型|主命令)：.*$", "", body, flags=re.M)
    body = re.sub(r"^\s*`[a-z][a-z0-9_]+`\s*$", "", body, flags=re.M)
    body = re.sub(r"[ \t]+$", "", body, flags=re.M)
    body = re.sub(r"\n{3,}", "\n\n", body).strip()
    return body


def rendered_content_hash(text: str) -> str:
    return hashlib.sha256(normalized_topic_body(text).encode("utf-8")).hexdigest()


def source_fragment_id(path: Path) -> str:
    match = SOURCE_FRAGMENT_RE.search(path.name)
    return match.group(1).lower() if match else ""


def is_applicability_topic(frontmatter: dict, path: Path) -> bool:
    return bool(
        APPLICABLE_TITLE_RE.search(frontmatter.get("title", ""))
        or re.search(r"applicable_(?:model|module)", path.name, re.I)
    )


def map_scope_catalog(topic_records: list[dict]) -> dict[str, dict]:
    """Resolve map scope from its applicability topic, falling back only to the map filename."""
    declared = defaultdict(set)
    for record in topic_records:
        if record["map_id"] and is_applicability_topic(record["fm"], record["path"]):
            declared[record["map_id"]].update(infer_model_codes(record["body"]))

    catalog = {}
    for record in topic_records:
        map_id = record["map_id"]
        if not map_id or map_id in catalog:
            continue
        models = sorted(declared.get(map_id, set()))
        source = "explicit_table" if models else "unknown"
        if not models:
            models = infer_model_codes(record["fm"].get("map_file", ""))
            if models:
                source = "map_name_fallback"
        catalog[map_id] = {"models": models, "scope_source": source}
    return catalog


def syntax_conflicts(command_catalog: list[dict]) -> tuple[list[dict], list[dict]]:
    """Separate publication defects from real product-branch disagreements.

    A single DITAMAP that contradicts itself is a data defect and must block the
    build.  Two maps that both claim the same model — e.g. the standard HTTP manual
    and its telecom-customised branch — is a genuine difference that a human has to
    adjudicate, so it is reported rather than silently merged into one command.
    """
    within_map: dict[tuple, dict[str, set[str]]] = defaultdict(lambda: defaultdict(set))
    across_maps: dict[tuple, dict[str, dict[str, set[str]]]] = defaultdict(
        lambda: defaultdict(lambda: defaultdict(set))
    )
    for row in command_catalog:
        if not row["syntax_hash"]:
            continue
        command = row["command"]
        map_id = row["map_id"]
        language = row["language"]
        syntax_hash = row["syntax_hash"]
        syntax = row.get("syntax", "")
        within_map[(command, map_id, language)][syntax_hash].add(syntax)
        for model in row["models"]:
            scope_key = (command, normalize_model_code(model), language)
            across_maps[scope_key][map_id][syntax_hash].add(syntax)

    same_map = []
    for (command, map_id, language), variants in sorted(within_map.items()):
        if len(variants) < 2:
            continue
        same_map.append({
            "command": command,
            "mapId": map_id,
            "language": language,
            "syntaxCount": len(variants),
            "variants": [
                {
                    "syntaxHash": syntax_hash,
                    "syntaxes": sorted(syntax for syntax in syntaxes if syntax),
                }
                for syntax_hash, syntaxes in sorted(variants.items())
            ],
        })

    cross_map = []
    for (command, model, language), by_map in sorted(across_maps.items()):
        # Do not repeat an internally inconsistent map as a cross-publication
        # disagreement.  Once that publication defect is fixed, a later build can
        # compare its one authoritative syntax with the other maps.
        consistent_maps = {
            map_id: variants
            for map_id, variants in by_map.items()
            if len(variants) == 1
        }
        if len(consistent_maps) < 2:
            continue
        variants_by_hash: dict[str, dict[str, object]] = {}
        for map_id, variants in consistent_maps.items():
            syntax_hash, syntaxes = next(iter(variants.items()))
            variant = variants_by_hash.setdefault(
                syntax_hash,
                {"syntaxHash": syntax_hash, "syntaxes": set(), "mapIds": []},
            )
            variant["syntaxes"].update(syntax for syntax in syntaxes if syntax)
            variant["mapIds"].append(map_id)
        if len(variants_by_hash) < 2:
            continue
        variant_rows = []
        for syntax_hash in sorted(variants_by_hash):
            variant = variants_by_hash[syntax_hash]
            variant_rows.append({
                "syntaxHash": syntax_hash,
                "syntaxes": sorted(variant["syntaxes"]),
                "mapIds": sorted(variant["mapIds"]),
            })
        cross_map.append({
            "command": command,
            "model": model,
            "language": language,
            "syntaxCount": len(variant_rows),
            "mapIds": sorted(consistent_maps),
            "variants": variant_rows,
        })
    return same_map, cross_map


def owner_for(record: dict, scope: dict) -> dict:
    fm = record["fm"]
    return {
        "map_id": record["map_id"],
        "repo": fm.get("repo", ""),
        "map_file": fm.get("map_file", ""),
        "manual": fm.get("manual_title", ""),
        "models": list(scope.get("models", [])),
        "scope_source": scope.get("scope_source", "unknown"),
        "language": record.get("language") or topic_language(fm, record["body"]),
        "source_file": fm.get("source_file", ""),
        "path": record["rel_path"],
    }


def _harvest_manuals_legacy(
    root: Path,
    out_dir: Path,
    index_dir: Path,
    install_map: bool = False,
    pack_id: str = "cmiot.manuals.fulltext",
    pack_version: Optional[str] = None,
):
    """
    扫描 manuals/*/topics/*.md：
      - kind == "AT command" 的主题 → 收割为指令文档 data/at_docs/manuals_harvest.yaml
      - 全部主题 → 公开检索索引 data/kb_index/manual_topics_public.jsonl（供 AI search_manuals 工具）
    """
    commands = {}     # cmd -> entry（去重：保留 doc 最长的版本，记录出现的手册）
    index_lines = []
    path_map_lines = []
    topics = list(root.glob("*/topics/*.md"))
    print(f"扫描 {len(topics)} 个主题文件…")

    for p in topics:
        try:
            text = p.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        fm = parse_frontmatter(text)
        title = fm.get("title", p.stem)
        manual = fm.get("manual_title", "")
        kind = fm.get("kind", "")
        body = FM_RE.sub("", text, count=1)
        models = infer_model_codes(manual)

        # 在压平前移除元数据行（手册/仓库/来源/类型），避免仓库标识泄漏到公开索引
        body = re.sub(r"^[-\s]*(手册|仓库|来源|类型)：.*$", "", body, flags=re.M)
        # 移除獨立的仓库标识行（如 `5glanfunctionusageguide`）
        body = re.sub(r"^\s*`[a-z][a-z0-9_]+`\s*$", "", body, flags=re.M)

        # 1. 检索索引（标题+手册名+预览，正文运行时按需读盘）
        preview = re.sub(r"\s+", " ", body)[:240]
        rel_path = str(p.relative_to(root)).replace("\\", "/")
        tid = topic_id_for(rel_path, title)
        index_lines.append(json.dumps({
            "topic_id": tid,
            "title": title, "manual": manual, "kind": kind, "preview": preview,
            "models": models,
            "variants": infer_topic_variants(rel_path),
        }, ensure_ascii=False))
        # 内部路径映射（不随应用打包）
        path_map_lines.append(json.dumps({
            "topic_id": tid, "path": rel_path,
        }, ensure_ascii=False))

        # 2. AT 指令主题收割
        if kind.lower() != "at command":
            continue
        m = CMD_IN_TITLE.search(title)
        if not m:
            continue
        cmd = m.group(1).upper()
        params = clean_text(extract_section(body, "参数描述"))
        shortdesc = ""
        bq = re.search(r"^> (.+)$", body, re.M)
        if bq:
            shortdesc = bq.group(1).strip()
        doc_parts = [shortdesc]
        if params:
            doc_parts.append("参数：\n" + params)
        doc = "\n\n".join(x for x in doc_parts if x)
        if len(doc) > MAX_DOC_LEN:
            doc = doc[:MAX_DOC_LEN] + "…(截断)"
        syntax = ""
        syn_sec = extract_section(body, "语法与响应")
        sm = re.search(rf"^({re.escape(cmd)}=[^\n]*)$", syn_sec, re.M | re.I)
        if sm:
            syntax = sm.group(1)

        source = f'{manual} ({fm.get("source_file", "")})'
        entry = {
            "cmd": cmd if cmd.startswith(("AT", "+")) else "AT" + cmd,
            "syntax": syntax,
            "category": fm.get("repo", ""),
            "kind": "urc" if cmd.startswith("+") else "command",
            "risk": "none",
            "title": title,
            "doc": doc,
            "examples": extract_examples(body, 1),
            "source": source,
            "models": models,
            "sources": [source],
            "variants": infer_topic_variants(rel_path),
        }
        old = commands.get(cmd)
        if old is None:
            commands[cmd] = entry
        else:
            merged_models = sorted(set(old["models"]) | set(entry["models"]))
            merged_sources = list(dict.fromkeys(old["sources"] + entry["sources"]))
            old_variants = old.get("variants", [])
            entry_variants = entry.get("variants", [])
            merged_variants = sorted(set(old_variants) | set(entry_variants)) if old_variants and entry_variants else []
            if len(entry["doc"]) > len(old["doc"]):
                commands[cmd] = entry
            commands[cmd]["models"] = merged_models
            commands[cmd]["sources"] = merged_sources
            commands[cmd]["variants"] = merged_variants

    # 落盘
    out_dir.mkdir(parents=True, exist_ok=True)
    index_dir.mkdir(parents=True, exist_ok=True)

    raw_index_dir = Path("tools/kb_index")
    raw_index_dir.mkdir(parents=True, exist_ok=True)
    with open(raw_index_dir / "manual_topics.jsonl", "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(index_lines))
    with open(raw_index_dir / "topic_paths.jsonl", "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(path_map_lines))

    public_lines = []
    for line in index_lines:
        topic = json.loads(line)
        topic["preview"] = sanitize_preview(topic.get("preview", ""))
        public_lines.append(json.dumps(topic, ensure_ascii=False))
    with open(index_dir / "manual_topics_public.jsonl", "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(public_lines))

    # 输出完整知识包目录（发布版用户选为 manuals_dir 后可启用全文）
    if install_map:
        with open(root / "topic_paths.jsonl", "w", encoding="utf-8", newline="\n") as f:
            f.write("\n".join(path_map_lines))
        # 生成 manifest.json，使该目录成为可验证的完整知识包
        import datetime
        resolved_id = pack_id.strip()
        resolved_version = (pack_version or datetime.datetime.now().strftime("%Y.%m")).strip()
        if not resolved_id or not resolved_version:
            raise ValueError("知识包 ID 和版本不能为空")
        manifest = {
            "schema": "modulink/knowledge-pack@1",
            "id": resolved_id,
            "version": resolved_version,
            "models": sorted({m for line in index_lines for m in json.loads(line).get("models", [])}),
            "generatedAt": datetime.datetime.now().strftime("%Y-%m-%d"),
            "topicCount": len(path_map_lines),
        }
        with open(root / "manifest.json", "w", encoding="utf-8", newline="\n") as f:
            json.dump(manifest, f, ensure_ascii=False, indent=2)
        print(f"路径映射安装到 {len(path_map_lines)} 条 → {root / 'topic_paths.jsonl'}")
        print(f"manifest → {root / 'manifest.json'}")
    try:
        import yaml

        def dump(obj, f):
            yaml.safe_dump(obj, f, allow_unicode=True, sort_keys=False, width=100)
    except ImportError:
        def dump(obj, f):
            json.dump(obj, f, ensure_ascii=False, indent=2)

    out = {
        "schema": "modulink/at_docs@1",
        "protocol": "MANUALS",
        "applies_to": [],
        "source": "anythingllm_kb/manuals 主题收割 (kind=AT command)",
        "commands": sorted(commands.values(), key=lambda c: c["cmd"]),
    }
    with open(out_dir / "manuals_harvest.yaml", "w", encoding="utf-8", newline="\n") as f:
        dump(out, f)
    print(f"收割 AT 指令 {len(commands)} 条 → {out_dir / 'manuals_harvest.yaml'}")
    print(f"原始索引 {len(index_lines)} 条 → {raw_index_dir / 'manual_topics.jsonl'}")
    print(f"路径映射 {len(path_map_lines)} 条 → {raw_index_dir / 'topic_paths.jsonl'}")
    print(f"公开索引 {len(public_lines)} 条 → {index_dir / 'manual_topics_public.jsonl'}")


def harvest_manuals(
    root: Path,
    out_dir: Path,
    index_dir: Path,
    install_map: bool = False,
    pack_id: str = "cmiot.manuals.fulltext",
    pack_version: Optional[str] = None,
    strict_quality: bool = False,
    pack_models: Optional[list[str]] = None,
    include_english: bool = False,
):
    """Compile DITA Markdown into V3 indexes, emitting Chinese topics by default.

    All source records still participate in map-scope resolution because an
    English ``Applicable Model`` topic may be the only structured scope evidence
    for Chinese topics in the same DITAMAP.
    """
    root = root.resolve()
    out_dir = out_dir.resolve()
    index_dir = index_dir.resolve()
    validate_manual_source_root(root)
    for label, destination in (("--out", out_dir), ("--index", index_dir)):
        if path_is_within(destination, root):
            raise ValueError(f"{label} 不能位于 --root 手册源目录内: {destination}")

    scanned_records = []
    for path in sorted(root.glob("*/topics/*.md")):
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        fm = parse_frontmatter(text)
        rel_path = str(path.relative_to(root)).replace("\\", "/")
        body = normalized_topic_body(text)
        scanned_records.append({
            "path": path,
            "rel_path": rel_path,
            "text": text,
            "body": body,
            "fm": fm,
            "map_id": map_id_for(fm),
            "content_hash": rendered_content_hash(text),
            "source_fragment_id": source_fragment_id(path),
            "language": topic_language(fm, body),
        })
    print(f"扫描 {len(scanned_records)} 个主题文件…")

    # Scope must be compiled before the language filter.  Some maps carry their
    # only model declaration in an English applicability topic.
    scopes = map_scope_catalog(scanned_records)
    records = (
        scanned_records
        if include_english
        else [record for record in scanned_records if record["language"] != "en"]
    )
    excluded_english_topics = len(scanned_records) - len(records)
    canonical = {}
    fragment_hashes = defaultdict(set)
    for record in records:
        content_hash = record["content_hash"]
        fragment_id = record["source_fragment_id"]
        if fragment_id:
            fragment_hashes[fragment_id].add(content_hash)
        scope = scopes.get(record["map_id"], {"models": [], "scope_source": "unknown"})
        owner = owner_for(record, scope)
        entry = canonical.get(content_hash)
        if entry is None:
            fm = record["fm"]
            entry = {
                "topic_id": content_hash[:16],
                "content_hash": content_hash,
                "source_fragment_ids": [],
                "title": fm.get("title", record["path"].stem),
                "manual": fm.get("manual_title", ""),
                "kind": fm.get("kind", ""),
                "preview": re.sub(r"\s+", " ", record["body"])[:240],
                "models": [],
                "scope_state": "unknown",
                "language": record["language"],
                "owners": [],
                "variants": [],
                "commands": set(),
                "record": record,
            }
            canonical[content_hash] = entry
        if owner not in entry["owners"]:
            entry["owners"].append(owner)
        if fragment_id and fragment_id not in entry["source_fragment_ids"]:
            entry["source_fragment_ids"].append(fragment_id)
        entry["variants"] = sorted(
            set(entry["variants"]) | set(infer_topic_variants(record["rel_path"]))
        )
        command = record["fm"].get("command", "").strip()
        match = CMD_IN_TITLE.search(record["fm"].get("title", ""))
        if not command and match:
            command = match.group(1)
        if command:
            command = command.upper()
            entry["commands"].add(command if command.startswith(("AT", "+")) else "AT" + command)

    index_rows = []
    path_rows = []
    for entry in canonical.values():
        entry["owners"].sort(key=lambda item: (item["map_id"], item["language"], item["manual"]))
        entry["models"] = sorted({model for owner in entry["owners"] for model in owner["models"]})
        entry["scope_state"] = (
            "resolved" if any(owner["scope_source"] != "unknown" for owner in entry["owners"])
            else "unknown"
        )
        row = {key: value for key, value in entry.items() if key not in {"record", "commands"}}
        row["commands"] = sorted(entry["commands"])
        row["preview"] = sanitize_preview(row["preview"])
        index_rows.append(row)
        path_rows.append({"topic_id": entry["topic_id"], "path": entry["record"]["rel_path"]})
    index_rows.sort(key=lambda row: row["topic_id"])
    path_rows.sort(key=lambda row: row["topic_id"])

    command_catalog = []
    at_docs = []
    seen_variants = set()
    for entry in canonical.values():
        if entry["kind"].lower() != "at command" or not entry["commands"]:
            continue
        body = entry["record"]["body"]
        unsupported_models = extract_unsupported_command_models(body)
        params = clean_text(extract_section(body, "参数描述"))
        quote = re.search(r"^> (.+)$", body, re.M)
        doc_parts = [quote.group(1).strip() if quote else ""]
        if params:
            doc_parts.append("参数：\n" + params)
        doc = "\n\n".join(part for part in doc_parts if part)[:MAX_DOC_LEN]
        for command in sorted(entry["commands"]):
            syntax_section = extract_section(body, "语法与响应")
            syntax_lines = re.findall(
                rf"^({re.escape(command)}[^\n]*)$", syntax_section, re.M | re.I
            )
            syntax = pick_syntax(syntax_lines)
            normalized_syntax = re.sub(r"\s+", "", syntax.upper())
            syntax_hash = hashlib.sha256(normalized_syntax.encode()).hexdigest() if syntax else ""
            for owner in entry["owners"]:
                parameter_constraints = extract_parameter_constraints(body, owner["models"])
                variant_key = (command, owner["map_id"], owner["language"], entry["content_hash"])
                if variant_key in seen_variants:
                    continue
                seen_variants.add(variant_key)
                catalog_row = {
                    "command": command,
                    "topic_id": entry["topic_id"],
                    "content_hash": entry["content_hash"],
                    "syntax_hash": syntax_hash,
                    "syntax": syntax,
                    "title": entry["title"],
                    "manual": owner["manual"],
                    "map_id": owner["map_id"],
                    "models": owner["models"],
                    "scope_source": owner["scope_source"],
                    "language": owner["language"],
                    "variants": entry["variants"],
                    "unsupported_models": unsupported_models,
                    "parameter_constraints": parameter_constraints,
                }
                command_catalog.append(catalog_row)
                source = f'{owner["manual"]} ({owner["source_file"]})'
                at_docs.append({
                    "cmd": command,
                    "syntax": syntax,
                    "category": owner["repo"],
                    "kind": "urc" if command.startswith("+") else "command",
                    "risk": "none",
                    "title": entry["title"],
                    "doc": doc,
                    "examples": extract_examples(body, 1),
                    "source": source,
                    "models": owner["models"],
                    "sources": [source],
                    "variants": entry["variants"],
                    "topic_id": entry["topic_id"],
                    "map_id": owner["map_id"],
                    "scope_source": owner["scope_source"],
                    "language": owner["language"],
                    "content_hash": entry["content_hash"],
                })

    command_catalog.sort(
        key=lambda row: (row["command"], row["map_id"], row["language"], row["content_hash"])
    )
    at_docs.sort(key=lambda row: (row["cmd"], row["map_id"], row["language"], row["content_hash"]))
    capability_restrictions = []
    for row in command_catalog:
        for model in row["unsupported_models"]:
            capability_restrictions.append({
                "kind": "unsupported_command",
                "command": row["command"],
                "model": model,
                "manual": row["manual"],
                "map_id": row["map_id"],
                "topic_id": row["topic_id"],
                "language": row["language"],
            })
        for constraint in row["parameter_constraints"]:
            for model in constraint["models"]:
                capability_restrictions.append({
                    "kind": "parameter_" + constraint["kind"],
                    "command": row["command"],
                    "model": model,
                    "parameter": constraint["parameter"],
                    "selector": constraint["selector"],
                    "values": constraint["values"],
                    "constraint_kind": constraint["kind"],
                    "confidence": constraint["confidence"],
                    "enforce": constraint["enforce"],
                    "note": constraint["note"],
                    "manual": row["manual"],
                    "map_id": row["map_id"],
                    "topic_id": row["topic_id"],
                    "language": row["language"],
                })
    capability_restrictions.sort(
        key=lambda row: (row["model"], row["command"], row["kind"], row.get("parameter", ""), row["map_id"])
    )
    same_map_conflicts, cross_map_conflicts = syntax_conflicts(command_catalog)
    unknown_topics = sum(row["scope_state"] == "unknown" for row in index_rows)
    quality = {
        "schema": "modulink/knowledge-quality@3",
        "scannedSourceTopicCount": len(scanned_records),
        "excludedEnglishTopicCount": excluded_english_topics,
        "sourceTopicCount": len(records),
        "canonicalTopicCount": len(index_rows),
        "deduplicatedTopicCount": len(records) - len(index_rows),
        "mapUnitCount": len(scopes),
        "resolvedMapUnitCount": sum(scope["scope_source"] != "unknown" for scope in scopes.values()),
        "unknownTopicCount": unknown_topics,
        "unknownTopicRatio": round(unknown_topics / len(index_rows), 4) if index_rows else 0,
        "sourceFragmentCollisionCount": sum(len(hashes) > 1 for hashes in fragment_hashes.values()),
        "commandVariantCount": len(command_catalog),
        "sameMapSyntaxConflictCount": len(same_map_conflicts),
        "crossMapModelSyntaxConflictCount": len(cross_map_conflicts),
        "capabilityRestrictionCount": len(capability_restrictions),
        "unsupportedCommandRestrictionCount": sum(
            row["kind"] == "unsupported_command" for row in capability_restrictions
        ),
        "parameterValueRestrictionCount": sum(
            row["kind"].startswith("parameter_") for row in capability_restrictions
        ),
        "sameMapSyntaxConflicts": same_map_conflicts,
        "crossMapModelSyntaxConflicts": cross_map_conflicts,
    }

    out_dir.mkdir(parents=True, exist_ok=True)
    index_dir.mkdir(parents=True, exist_ok=True)
    raw_index_dir = Path("tools/kb_index")
    raw_index_dir.mkdir(parents=True, exist_ok=True)
    index_lines = [json.dumps(row, ensure_ascii=False) for row in index_rows]
    path_lines = [json.dumps(row, ensure_ascii=False) for row in path_rows]
    public_rows = []
    for row in index_rows:
        public = dict(row)
        public["owners"] = [
            {
                key: owner[key]
                for key in ("map_id", "manual", "models", "scope_source", "language")
            }
            for owner in row["owners"]
        ]
        public_rows.append(public)
    public_lines = [json.dumps(row, ensure_ascii=False) for row in public_rows]
    for destination in (raw_index_dir / "manual_topics.jsonl", raw_index_dir / "topic_meta.jsonl"):
        write_text_lf(destination, "\n".join(index_lines) + "\n")
    write_text_lf(raw_index_dir / "topic_paths.jsonl", "\n".join(path_lines) + "\n")
    write_text_lf(index_dir / "manual_topics_public.jsonl", "\n".join(public_lines) + "\n")
    write_text_lf(
        index_dir / "command_catalog.jsonl",
        "\n".join(json.dumps(row, ensure_ascii=False) for row in command_catalog) + "\n",
    )
    write_text_lf(
        index_dir / "knowledge_quality.json",
        json.dumps(quality, ensure_ascii=False, indent=2) + "\n",
    )
    write_text_lf(
        index_dir / "capability_restrictions.jsonl",
        "\n".join(json.dumps(row, ensure_ascii=False) for row in capability_restrictions) + "\n",
    )

    try:
        import yaml

        serialized = yaml.safe_dump(
            {
                "schema": "modulink/at_docs@2",
                "protocol": "MANUALS",
                "applies_to": [],
                "source": "anythingllm_kb/manuals DITAMAP scope compiler",
                "commands": at_docs,
            },
            allow_unicode=True,
            sort_keys=False,
            width=100,
        )
    except ImportError:
        serialized = json.dumps({"schema": "modulink/at_docs@2", "protocol": "MANUALS", "commands": at_docs}, ensure_ascii=False, indent=2)
    write_text_lf(out_dir / "manuals_harvest.yaml", serialized)

    strict_failures = []
    if strict_quality:
        if not scanned_records:
            strict_failures.append("未扫描到任何源主题")
        elif not records:
            strict_failures.append("语言筛选后没有可编译源主题")
        if not index_rows:
            strict_failures.append("canonical 主题索引为空")
        if same_map_conflicts:
            strict_failures.append(f"同一手册内命令语法自相矛盾 {len(same_map_conflicts)} 组")

    # Never install a manifest into the source tree after a failed strict build:
    # that would turn the source into something the self-reference guard rejects.
    if install_map and not strict_failures:
        resolved_version = (pack_version or datetime.datetime.now().strftime("%Y.%m")).strip()
        manifest_models = sorted({normalize_model_code(model) for model in (pack_models or [])})
        if not manifest_models:
            manifest_models = sorted({model for row in index_rows for model in row["models"]})
        selected = set(manifest_models)
        installed_counts = {"exact": 0, "series": 0, "unknown": 0}
        installed_topic_ids = set()
        for row in index_rows:
            scope = resolved_topic_scope(row, selected, True, True)
            if scope:
                installed_counts[scope] += 1
                installed_topic_ids.add(row["topic_id"])
        installed_path_lines = [
            json.dumps(row, ensure_ascii=False)
            for row in path_rows
            if row["topic_id"] in installed_topic_ids
        ]
        write_text_lf(root / "topic_paths.jsonl", "\n".join(installed_path_lines) + "\n")
        manifest = {
            "schema": "modulink/knowledge-pack@3",
            "id": pack_id.strip(),
            "version": resolved_version,
            "models": manifest_models,
            "generatedAt": datetime.datetime.now().strftime("%Y-%m-%d"),
            "topicCount": len(installed_path_lines),
            "includeSeries": True,
            "includeUnknownReference": True,
            "topicCounts": installed_counts,
        }
        write_text_lf(
            root / "manifest.json",
            json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
        )

    print(
        f"canonical 主题 {len(index_rows)}（去重 {len(records) - len(index_rows)}；"
        f"排除英文 {excluded_english_topics}）"
    )
    print(f"命令 variant {len(command_catalog)}；unknown 主题 {unknown_topics}")
    if cross_map_conflicts:
        print(f"⚠ 跨手册同型号语法分歧 {len(cross_map_conflicts)} 组，需人工裁决：")
        for conflict in cross_map_conflicts[:10]:
            print(f"    {conflict['command']} @ {conflict['model']}: {'、'.join(conflict['mapIds'])}")
        if len(cross_map_conflicts) > 10:
            print(f"    …另有 {len(cross_map_conflicts) - 10} 组，详见质量报告")
    print(f"质量报告 → {index_dir / 'knowledge_quality.json'}")
    if strict_failures:
        raise ValueError("严格质量检查失败：" + "；".join(strict_failures) + "，详见质量报告")
    return quality


def read_jsonl(path: Path) -> list[dict]:
    rows = []
    seen_ids = set()
    with path.open("r", encoding="utf-8") as source:
        for line_no, line in enumerate(source, 1):
            if not line.strip():
                continue
            try:
                row = json.loads(line)
            except json.JSONDecodeError as exc:
                raise ValueError(f"{path}:{line_no} JSON 格式错误: {exc}") from exc
            topic_id = str(row.get("topic_id", "")).strip()
            if not topic_id:
                raise ValueError(f"{path}:{line_no} 缺少 topic_id")
            if topic_id in seen_ids:
                raise ValueError(f"{path}:{line_no} topic_id 重复: {topic_id}")
            seen_ids.add(topic_id)
            rows.append(row)
    return rows


def export_model_pack(
    root: Path,
    public_index: Path,
    path_map: Path,
    output: Path,
    models: list[str],
    pack_id: str,
    pack_version: Optional[str],
    include_series: bool,
    include_unknown_reference: bool,
    create_zip: bool,
):
    """Export an immutable @3 bundle containing only the declared model scope."""
    root = root.resolve()
    output = output.resolve()
    validate_manual_source_root(root)
    selected_models = {normalize_model_code(model) for model in models if model.strip()}
    if not selected_models:
        raise ValueError("至少需要指定一个型号")
    if path_is_within(output, root):
        raise ValueError(f"知识包输出目录不能位于 manuals 源目录内: {output}")
    if output.exists():
        raise ValueError(f"输出目录已存在，请使用新的版本目录: {output}")
    if not public_index.is_file() or not path_map.is_file():
        raise ValueError("公开索引或内部路径映射不存在，请先运行 manuals 收割")

    index_rows = read_jsonl(public_index)
    path_rows = read_jsonl(path_map)
    paths_by_id = {row["topic_id"]: str(row.get("path", "")) for row in path_rows}
    indexed_models = {
        normalize_model_code(model)
        for topic in index_rows
        for model in topic.get("models", [])
        if str(model).strip()
    }
    unknown_models = sorted(selected_models - indexed_models)
    if unknown_models:
        raise ValueError(f"公开索引未收录型号: {', '.join(unknown_models)}")

    selected_rows = []
    scope_counts = {"exact": 0, "series": 0, "unknown": 0}
    for topic in index_rows:
        kind = resolved_topic_scope(
            topic, selected_models, include_series, include_unknown_reference
        )
        if kind:
            selected_rows.append(topic)
            scope_counts[kind] += 1
    if not selected_rows:
        raise ValueError("所选型号作用域没有可导出的主题")

    copy_plan = []
    for topic in selected_rows:
        topic_id = topic["topic_id"]
        rel_text = paths_by_id.get(topic_id, "")
        rel = PurePosixPath(rel_text)
        if not rel_text or rel.is_absolute() or ".." in rel.parts or rel.suffix.lower() != ".md":
            raise ValueError(f"topic {topic_id} 的路径不安全或缺失: {rel_text}")
        source = root.joinpath(*rel.parts).resolve()
        try:
            source.relative_to(root)
        except ValueError as exc:
            raise ValueError(f"topic {topic_id} 路径逃逸知识库根目录: {rel_text}") from exc
        if not source.is_file():
            raise ValueError(f"topic {topic_id} 正文不存在: {source}")
        copy_plan.append((topic_id, rel_text, source, rel))

    output.parent.mkdir(parents=True, exist_ok=True)
    resolved_id = pack_id.strip()
    resolved_version = (pack_version or datetime.datetime.now().strftime("%Y.%m")).strip()
    if not resolved_id or not resolved_version:
        raise ValueError("知识包 ID 和版本不能为空")

    with tempfile.TemporaryDirectory(prefix=f".{output.name}-", dir=output.parent) as temp_dir:
        staging = Path(temp_dir)
        mappings = []
        for topic_id, rel_text, source, rel in copy_plan:
            destination = staging.joinpath(*rel.parts)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
            mappings.append(json.dumps({"topic_id": topic_id, "path": rel_text}, ensure_ascii=False))

        manifest = {
            "schema": "modulink/knowledge-pack@3",
            "id": resolved_id,
            "version": resolved_version,
            "models": sorted(selected_models),
            "generatedAt": datetime.datetime.now().strftime("%Y-%m-%d"),
            "topicCount": len(mappings),
            "includeSeries": include_series,
            "includeUnknownReference": include_unknown_reference,
            "topicCounts": scope_counts,
        }
        write_text_lf(staging / "topic_paths.jsonl", "\n".join(mappings) + "\n")
        write_text_lf(
            staging / "manifest.json",
            json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
        )
        try:
            staging.rename(output)
        except PermissionError:
            # 部分 Windows 受控目录允许创建/复制，但拒绝目录 rename。
            # output 在函数入口已保证不存在，因此可安全回退为 copytree。
            try:
                shutil.copytree(staging, output)
            except Exception:
                if output.exists():
                    shutil.rmtree(output)
                raise

    print(
        f"型号知识包 → {output}（精确 {scope_counts['exact']}，系列 {scope_counts['series']}，"
        f"unknown 参考 {scope_counts['unknown']}，合计 {len(copy_plan)}）"
    )
    if create_zip:
        archive = Path(shutil.make_archive(str(output), "zip", output.parent, output.name))
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        checksum = archive.with_suffix(archive.suffix + ".sha256")
        write_text_lf(checksum, f"{digest}  {archive.name}\n", encoding="ascii")
        print(f"ZIP → {archive}")
        print(f"SHA-256 → {checksum}")


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="mode")

    p1 = sub.add_parser("commands", help="转换 at_commands 结构化知识库")
    p1.add_argument("--kb", required=True)
    p1.add_argument("--out", default="data/at_docs")

    p2 = sub.add_parser("manuals", help="收割 manuals 主题（AT 指令 + 检索索引）")
    p2.add_argument("--root", required=True, help="manuals 根目录")
    p2.add_argument("--out", default="data/at_docs")
    p2.add_argument("--index", default="data/kb_index")
    p2.add_argument("--install-map", action="store_true",
        help="将 topic_paths.jsonl 安装到 --root 指向的手册根目录，这样用户选择该目录为 manuals_dir 后可启用全文")
    p2.add_argument("--pack-id", default="cmiot.manuals.fulltext",
        help="知识包 ID（仅 --install-map 时写入 manifest）")
    p2.add_argument("--pack-version",
        help="知识包版本（仅 --install-map 时使用；默认当前年月 YYYY.MM）")
    p2.add_argument("--models", nargs="+",
        help="安装为型号包时声明的精确型号；同时只安装精确、同系列和 unknown 参考主题映射")
    p2.add_argument("--strict-quality", action="store_true",
        help="空输入、空索引或同一手册内语法冲突时让构建失败（质量报告始终生成）")
    p2.add_argument(
        "--include-english",
        action="store_true",
        help="同时编译英文主题；默认仅输出中文，但英文适用型号主题仍参与范围解析",
    )

    p3 = sub.add_parser("usecases", help="转换通信流程示例为典型用例流程")
    p3.add_argument("--root", required=True, help="manuals 根目录")
    p3.add_argument("--out", default="data/usecases")

    p4 = sub.add_parser("export-pack", help="按型号导出可独立交付的 @3 全文知识包")
    p4.add_argument("--root", required=True, help="完整 manuals 根目录")
    p4.add_argument("--models", nargs="+", required=True, help="需要交付的一个或多个精确型号")
    p4.add_argument("--output", required=True, help="新的知识包输出目录（必须不存在）")
    p4.add_argument("--index", default="data/kb_index/manual_topics_public.jsonl", help="公开主题索引")
    p4.add_argument("--path-map", default="tools/kb_index/topic_paths.jsonl", help="内部主题路径映射")
    p4.add_argument("--pack-id", default="cmiot.manuals.customer-bundle")
    p4.add_argument("--pack-version", help="知识包版本，默认当前年月 YYYY.MM")
    p4.add_argument("--no-series", action="store_true", help="不包含同系列参考主题")
    p4.add_argument(
        "--no-unknown-reference",
        "--no-generic",
        dest="no_unknown_reference",
        action="store_true",
        help="不包含尚未解析出型号范围的参考主题（--no-generic 为旧名，仍可用）",
    )
    p4.add_argument("--zip", action="store_true", help="同时生成 ZIP 和 SHA-256 校验文件")

    # 兼容旧用法: 无子命令时按 commands 处理
    ap.add_argument("--kb", help=argparse.SUPPRESS)
    ap.add_argument("--out", default="data/at_docs", help=argparse.SUPPRESS)

    args = ap.parse_args()
    if args.mode == "usecases":
        convert_usecases(Path(args.root), Path(args.out))
    elif args.mode == "export-pack":
        export_model_pack(
            Path(args.root),
            Path(args.index),
            Path(args.path_map),
            Path(args.output),
            args.models,
            args.pack_id,
            args.pack_version,
            not args.no_series,
            not args.no_unknown_reference,
            args.zip,
        )
    elif args.mode == "manuals":
        harvest_manuals(
            Path(args.root),
            Path(args.out),
            Path(args.index),
            args.install_map,
            args.pack_id,
            args.pack_version,
            args.strict_quality,
            args.models,
            args.include_english,
        )
    else:
        kb = getattr(args, "kb", None)
        if not kb:
            sys.exit("用法: kb_convert.py commands --kb <dir> | kb_convert.py manuals --root <dir>")
        stats = convert(Path(kb), Path(args.out))
        print(f"完成: 共 {stats['total']} 条, 缺 md 正文 {stats['no_md']} 条")


if __name__ == "__main__":
    main()
