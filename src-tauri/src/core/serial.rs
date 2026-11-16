//! 串口引擎：端口枚举、多口并发 Session、行组装、Frame 事件推送、AT 命令队列。
//!
//! 数据流：串口字节 → 行组装（半包超时冲刷）→ Frame 分类（tx/rx/final/urc/raw）
//! → (1) 批量 emit 到前端 `serial://frames`；(2) 若有挂起的 AT 命令则同时路由给它收集。

use modulink_core::at_protocol::{classify_final, decode_hex, is_likely_urc};
use modulink_core::response_slot::ResponseSlot;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use tauri::Emitter;

static FRAME_SEQ: AtomicU64 = AtomicU64::new(0);
static URC_WAITER_SEQ: AtomicU64 = AtomicU64::new(0);
static SESSION_GENERATION: AtomicU64 = AtomicU64::new(1);
const MAX_HISTORY_FRAMES: usize = 100_000;
const EVENT_BATCH_SIZE: usize = 64;
const EVENT_BATCH_MS: u64 = 25;

#[derive(Debug, Clone, Serialize)]
pub struct PortInfo {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenPortInfo {
    pub port: String,
    pub baud: u32,
    pub generation: u64,
}

/// 贯穿全系统的统一帧结构。
#[derive(Debug, Clone, Serialize)]
pub struct Frame {
    pub seq: u64,
    /// Unix 毫秒
    pub ts: i64,
    pub port: String,
    pub generation: u64,
    /// "tx" | "rx"
    pub dir: String,
    /// echo | response | final | urc | raw | tx
    pub kind: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hex: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AtResult {
    pub request: String,
    pub lines: Vec<String>,
    /// "OK" | "ERROR" | "+CME ERROR: n" | "TIMEOUT"
    pub final_code: String,
    pub elapsed_ms: u64,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn make_frame(
    port: &str,
    generation: u64,
    dir: &str,
    kind: &str,
    text: String,
    hex: Option<String>,
) -> Frame {
    Frame {
        seq: FRAME_SEQ.fetch_add(1, Ordering::Relaxed),
        ts: now_ms(),
        port: port.to_string(),
        generation,
        dir: dir.into(),
        kind: kind.into(),
        text,
        hex,
    }
}

struct PendingCommand {
    request: String,
    lines: Vec<String>,
    tx_done: mpsc::Sender<AtResult>,
    started: Instant,
}

/// URC 订阅：诊断引擎等待异步上报（如 +MIPOPEN: 0,0）用
struct UrcWaiter {
    id: u64,
    pattern: regex::Regex,
    tx: mpsc::Sender<String>,
}

pub struct Session {
    pub port_name: String,
    pub baud: u32,
    pub generation: u64,
    writer: Mutex<Box<dyn serialport::SerialPort>>,
    pending: Arc<Mutex<ResponseSlot<PendingCommand>>>,
    /// AT 命令队列互斥：同一时刻只允许一个来源在等待响应
    at_queue: Arc<tokio::sync::Mutex<()>>,
    stop: Arc<AtomicBool>,
    history: Arc<Mutex<VecDeque<Frame>>>,
    urc_waiters: Arc<Mutex<Vec<UrcWaiter>>>,
}

pub struct SerialManager {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

// Caller holds the manager lock across validation and removal.
fn take_session<T>(
    sessions: &mut HashMap<String, T>,
    port: &str,
    expected: Option<u64>,
    generation: impl Fn(&T) -> u64,
) -> Result<Option<T>, String> {
    if let (Some(expected), Some(session)) = (expected, sessions.get(port)) {
        if generation(session) != expected {
            return Err(format!("{port} 会话已变化，未关闭当前连接"));
        }
    }
    Ok(sessions.remove(port)) // Closing an already-removed connection is idempotent.
}

#[cfg(test)]
mod close_tests {
    use super::*;
    #[test]
    fn late_close_keeps_the_reconnected_session() {
        let mut sessions = HashMap::from([("COM1".to_owned(), 8_u64)]);
        assert!(take_session(&mut sessions, "COM1", Some(7), |v| *v).is_err());
        assert_eq!(sessions.get("COM1"), Some(&8));
    }
    #[test]
    fn close_removes_only_the_matching_session_and_is_retryable() {
        let mut sessions = HashMap::from([("COM1".to_owned(), 7_u64), ("COM2".to_owned(), 9)]);
        assert_eq!(
            take_session(&mut sessions, "COM1", Some(7), |v| *v).unwrap(),
            Some(7)
        );
        assert_eq!(
            take_session(&mut sessions, "COM1", Some(7), |v| *v).unwrap(),
            None
        );
        assert_eq!(sessions.get("COM2"), Some(&9));
    }
}

impl Default for SerialManager {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }
}

impl SerialManager {
    pub fn list_ports() -> Vec<PortInfo> {
        serialport::available_ports()
            .unwrap_or_default()
            .into_iter()
            .map(|p| {
                let description = match &p.port_type {
                    serialport::SerialPortType::UsbPort(u) => {
                        u.product.clone().unwrap_or_else(|| "USB 串口".into())
                    }
                    serialport::SerialPortType::BluetoothPort => "蓝牙串口".into(),
                    serialport::SerialPortType::PciPort => "PCI 串口".into(),
                    _ => String::new(),
                };
                PortInfo {
                    name: p.port_name,
                    description,
                }
            })
            .collect()
    }

    pub fn open(
        &self,
        app: tauri::AppHandle,
        port_name: &str,
        baud: u32,
    ) -> Result<OpenPortInfo, String> {
        let mut sessions = self.sessions.lock();
        if let Some(existing) = sessions.get(port_name) {
            if existing.stop.load(Ordering::Relaxed) {
                sessions.remove(port_name);
            } else {
                return Err(format!("{port_name} 已打开"));
            }
        }
        let port = serialport::new(port_name, baud)
            .timeout(Duration::from_millis(50))
            .open()
            .map_err(|e| format!("打开 {port_name} 失败: {e}"))?;
        let writer = port.try_clone().map_err(|e| format!("克隆句柄失败: {e}"))?;

        let stop = Arc::new(AtomicBool::new(false));
        let pending = Arc::new(Mutex::new(ResponseSlot::default()));
        let history = Arc::new(Mutex::new(VecDeque::new()));
        let urc_waiters: Arc<Mutex<Vec<UrcWaiter>>> = Arc::new(Mutex::new(Vec::new()));
        let generation = SESSION_GENERATION.fetch_add(1, Ordering::Relaxed);
        let session = Arc::new(Session {
            port_name: port_name.to_string(),
            baud,
            generation,
            writer: Mutex::new(writer),
            pending: pending.clone(),
            at_queue: Arc::new(tokio::sync::Mutex::new(())),
            stop: stop.clone(),
            history: history.clone(),
            urc_waiters: urc_waiters.clone(),
        });
        sessions.insert(port_name.to_string(), session);
        drop(sessions);

        // 读线程
        let pn = port_name.to_string();
        std::thread::spawn(move || {
            reader_loop(
                app,
                port,
                pn,
                generation,
                pending,
                stop,
                history,
                urc_waiters,
            )
        });
        Ok(OpenPortInfo {
            port: port_name.to_string(),
            baud,
            generation,
        })
    }

    #[cfg(test)]
    pub fn close(&self, port_name: &str) -> Result<(), String> {
        self.close_matching(port_name, None)
    }

    pub fn close_generation(&self, port_name: &str, generation: u64) -> Result<(), String> {
        self.close_matching(port_name, Some(generation))
    }

    fn close_matching(&self, port_name: &str, generation: Option<u64>) -> Result<(), String> {
        let mut sessions = self.sessions.lock();
        if let Some(s) = take_session(&mut sessions, port_name, generation, |s| s.generation)? {
            s.stop.store(true, Ordering::Relaxed);
            finish_pending(&s.pending, "DISCONNECTED");
        }
        Ok(())
    }

    pub fn get(&self, port_name: &str) -> Result<Arc<Session>, String> {
        let mut sessions = self.sessions.lock();
        match sessions.get(port_name).cloned() {
            Some(session) if session.stop.load(Ordering::Relaxed) => {
                sessions.remove(port_name);
                Err(format!("{port_name} 未打开"))
            }
            Some(session) => Ok(session),
            None => Err(format!("{port_name} 未打开")),
        }
    }

    pub fn get_generation(&self, port_name: &str, generation: u64) -> Result<Arc<Session>, String> {
        let session = self.get(port_name)?;
        if session.generation != generation {
            return Err(format!("{port_name} 会话已变化，请基于当前设备重新执行"));
        }
        Ok(session)
    }

    pub fn open_ports(&self) -> Vec<OpenPortInfo> {
        let mut ports: Vec<OpenPortInfo> = self
            .sessions
            .lock()
            .iter()
            .filter(|(_, session)| !session.stop.load(Ordering::Relaxed))
            .map(|(port, session)| OpenPortInfo {
                port: port.clone(),
                baud: session.baud,
                generation: session.generation,
            })
            .collect();
        ports.sort_by(|a, b| a.port.cmp(&b.port));
        ports
    }
}

impl Session {
    /// 裸写（控制台手动发送、快捷指令用）。
    pub async fn write_raw(
        &self,
        app: &tauri::AppHandle,
        data: &str,
        is_hex: bool,
        append_newline: bool,
    ) -> Result<(), String> {
        let _guard = self.at_queue.lock().await;
        self.write_raw_locked(app, data, is_hex, append_newline)
    }

    fn write_raw_locked(
        &self,
        app: &tauri::AppHandle,
        data: &str,
        is_hex: bool,
        append_newline: bool,
    ) -> Result<(), String> {
        if self.stop.load(Ordering::Relaxed) {
            return Err("设备会话已关闭，请重新连接".into());
        }
        if self.pending.lock().is_draining() {
            return Err(
                "上条指令已超时，等待终结响应后才能继续发送；持续无响应时请复位设备后重连".into(),
            );
        }
        let bytes = if is_hex {
            decode_hex(data)?
        } else {
            let mut b = data.as_bytes().to_vec();
            if append_newline {
                b.extend_from_slice(b"\r\n");
            }
            b
        };
        if let Err(e) = self.writer.lock().write_all(&bytes) {
            let error = format!("写入失败，设备可能已断开: {e}");
            self.stop.store(true, Ordering::Relaxed);
            let _ = app.emit(
                "serial://error",
                serde_json::json!({ "port": self.port_name, "generation": self.generation, "error": error }),
            );
            return Err(error);
        }
        let frame = make_frame(
            &self.port_name,
            self.generation,
            "tx",
            "tx",
            if is_hex {
                String::new()
            } else {
                data.to_string()
            },
            if is_hex { Some(data.to_string()) } else { None },
        );
        record_frame(&self.history, frame.clone());
        emit_frames(app, &[frame]);
        Ok(())
    }

    pub async fn acquire_exclusive(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.at_queue.clone().lock_owned().await
    }

    pub(crate) async fn send_at_locked(
        &self,
        app: &tauri::AppHandle,
        cmd: &str,
        timeout_ms: u64,
    ) -> Result<AtResult, String> {
        let (tx_done, rx_done) = mpsc::channel::<AtResult>();
        {
            let mut p = self.pending.lock();
            if self.stop.load(Ordering::Relaxed) {
                return Err("设备会话已关闭，请重新连接".into());
            }
            p.begin(PendingCommand {
                request: cmd.to_string(),
                lines: Vec::new(),
                tx_done,
                started: Instant::now(),
            })
            .map_err(str::to_string)?;
        }
        if let Err(e) = self.write_raw_locked(app, cmd, false, true) {
            self.pending.lock().finish();
            return Err(e);
        }

        let started = Instant::now();
        let result = tokio::task::spawn_blocking(move || {
            let result =
                rx_done.recv_timeout(Duration::from_millis(timeout_ms.clamp(100, 120_000)));
            (result, rx_done)
        })
        .await
        .map_err(|e| e.to_string())?;

        let (result, rx_done) = result;
        match result {
            Ok(r) => Ok(r),
            Err(_) => {
                // 超时: 取走挂起状态, 返回已收集的行
                let taken = self.pending.lock().timeout();
                // The reader sends while holding this lock. If it won the race,
                // return the already-delivered result instead of a false timeout.
                if taken.is_none() {
                    if let Ok(completed) = rx_done.try_recv() {
                        return Ok(completed);
                    }
                }
                Ok(AtResult {
                    request: cmd.to_string(),
                    lines: taken.map(|p| p.lines).unwrap_or_default(),
                    final_code: "TIMEOUT".into(),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                })
            }
        }
    }

    pub fn history_snapshot(&self) -> Vec<Frame> {
        self.history.lock().iter().cloned().collect()
    }

    /// 预注册 URC 等待器，返回 (id, 接收端)。在发送 AT 指令前调用，避免
    /// OK 与注册之间的竞态丢事件。正则非法返回 None。
    pub fn arm_urc(&self, pattern: &str) -> Option<(u64, mpsc::Receiver<String>)> {
        let re = regex::Regex::new(pattern).ok()?;
        let id = URC_WAITER_SEQ.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel::<String>();
        self.urc_waiters.lock().push(UrcWaiter {
            id,
            pattern: re,
            tx,
        });
        Some((id, rx))
    }

    /// 按 id 移除 URC 等待器（超时或取消后主动清理，避免残留死 waiter）。
    pub fn remove_urc_waiter(&self, id: u64) {
        self.urc_waiters.lock().retain(|w| w.id != id);
    }
}

/// 读线程主循环：字节 → 行组装 → 分类 → emit + 路由给挂起命令。
fn reader_loop(
    app: tauri::AppHandle,
    mut port: Box<dyn serialport::SerialPort>,
    port_name: String,
    generation: u64,
    pending: Arc<Mutex<ResponseSlot<PendingCommand>>>,
    stop: Arc<AtomicBool>,
    history: Arc<Mutex<VecDeque<Frame>>>,
    urc_waiters: Arc<Mutex<Vec<UrcWaiter>>>,
) {
    let mut buf = [0u8; 4096];
    let mut acc: Vec<u8> = Vec::new();
    let mut last_rx = Instant::now();
    const PARTIAL_FLUSH_MS: u64 = 300;
    let mut event_batch: Vec<Frame> = Vec::new();
    let mut last_emit = Instant::now();

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match port.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => {
                acc.extend_from_slice(&buf[..n]);
                last_rx = Instant::now();
                // 逐行切分
                while let Some(pos) = acc.iter().position(|&b| b == b'\n' || b == b'\r') {
                    let line_bytes: Vec<u8> = acc.drain(..=pos).collect();
                    let line = String::from_utf8_lossy(&line_bytes)
                        .trim_end_matches(['\r', '\n'])
                        .to_string();
                    if line.is_empty() {
                        continue;
                    }
                    let frame = route_line(&port_name, generation, line, &pending);
                    notify_urc_waiters(&urc_waiters, &frame);
                    record_frame(&history, frame.clone());
                    event_batch.push(frame);
                }
                flush_event_batch(&app, &mut event_batch, &mut last_emit, false);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                // 半包超时冲刷（如无换行的提示符 ">"）
                if !acc.is_empty() && last_rx.elapsed() > Duration::from_millis(PARTIAL_FLUSH_MS) {
                    let line = String::from_utf8_lossy(&acc).to_string();
                    acc.clear();
                    if !line.trim().is_empty() {
                        let frame = route_line(&port_name, generation, line, &pending);
                        notify_urc_waiters(&urc_waiters, &frame);
                        record_frame(&history, frame.clone());
                        event_batch.push(frame);
                    }
                }
                flush_event_batch(&app, &mut event_batch, &mut last_emit, true);
            }
            Err(_) => {
                // 设备拔出等致命错误
                stop.store(true, Ordering::Relaxed);
                if let Some(p) = pending.lock().finish() {
                    let _ = p.tx_done.send(AtResult {
                        request: p.request,
                        lines: p.lines,
                        final_code: "DISCONNECTED".into(),
                        elapsed_ms: p.started.elapsed().as_millis() as u64,
                    });
                }
                let _ = app.emit(
                    "serial://error",
                    serde_json::json!({ "port": port_name, "generation": generation, "error": "读串口失败，设备可能已断开" }),
                );
                break;
            }
        }
    }
    flush_event_batch(&app, &mut event_batch, &mut last_emit, true);
}

fn route_line(
    port_name: &str,
    generation: u64,
    line: String,
    pending: &Arc<Mutex<ResponseSlot<PendingCommand>>>,
) -> Frame {
    let final_code = classify_final(&line);
    let mut kind = "raw";

    {
        let mut guard = pending.lock();
        if let Some(p) = guard.pending_mut() {
            // 回显行
            if line.trim() == p.request.trim() {
                kind = "echo";
            } else if let Some(fc) = &final_code {
                kind = "final";
                let result = AtResult {
                    request: p.request.clone(),
                    lines: p.lines.clone(),
                    final_code: fc.clone(),
                    elapsed_ms: p.started.elapsed().as_millis() as u64,
                };
                let _ = p.tx_done.send(result);
                guard.finish();
            } else if is_likely_urc(&p.request, &line) {
                kind = "urc";
            } else {
                kind = "response";
                p.lines.push(line.clone());
            }
        } else if final_code.is_some() {
            guard.finish(); // consume the timed-out command's late final
            kind = "final";
        } else if line.trim_start().starts_with('+') || line.trim_start().starts_with('*') {
            // 无挂起命令时收到的 +XXX 视为 URC
            kind = "urc";
        }
    }

    make_frame(port_name, generation, "rx", kind, line, None)
}

/// 命中的 waiter 收到该行并被移除（一次性消费）；命中但接收端已死
/// （等待方超时放弃）的 waiter 一并清理。未命中的保留。
fn notify_urc_waiters(waiters: &Arc<Mutex<Vec<UrcWaiter>>>, frame: &Frame) {
    if frame.dir != "rx" {
        return;
    }
    let mut list = waiters.lock();
    if list.is_empty() {
        return;
    }
    list.retain(|w| {
        if !w.pattern.is_match(&frame.text) {
            return true; // 未命中，继续等
        }
        let _ = w.tx.send(frame.text.clone());
        false // 命中即移除（无论接收端是否还活着）
    });
}

fn finish_pending(pending: &Arc<Mutex<ResponseSlot<PendingCommand>>>, final_code: &str) {
    if let Some(p) = pending.lock().finish() {
        let _ = p.tx_done.send(AtResult {
            request: p.request,
            lines: p.lines,
            final_code: final_code.into(),
            elapsed_ms: p.started.elapsed().as_millis() as u64,
        });
    }
}

fn record_frame(history: &Arc<Mutex<VecDeque<Frame>>>, frame: Frame) {
    let mut history = history.lock();
    history.push_back(frame);
    while history.len() > MAX_HISTORY_FRAMES {
        history.pop_front();
    }
}

fn emit_frames(app: &tauri::AppHandle, frames: &[Frame]) {
    if !frames.is_empty() {
        let _ = app.emit("serial://frames", frames);
    }
}

fn flush_event_batch(
    app: &tauri::AppHandle,
    batch: &mut Vec<Frame>,
    last_emit: &mut Instant,
    force: bool,
) {
    if batch.is_empty() {
        return;
    }
    if force
        || batch.len() >= EVENT_BATCH_SIZE
        || last_emit.elapsed() >= Duration::from_millis(EVENT_BATCH_MS)
    {
        emit_frames(app, batch);
        batch.clear();
        *last_emit = Instant::now();
    }
}

#[cfg(test)]
mod event_contract_tests {
    use super::*;
    #[test]
    fn transmitted_and_received_frames_keep_their_connection_generation() {
        let tx = make_frame("COM3", 7, "tx", "tx", "AT".into(), None);
        let pending = Arc::new(Mutex::new(ResponseSlot::default()));
        let rx = route_line("COM3", 8, "+CREG: 1".into(), &pending);
        assert_eq!(serde_json::to_value(tx).unwrap()["generation"], 7);
        assert_eq!(serde_json::to_value(rx).unwrap()["generation"], 8);
    }
}
