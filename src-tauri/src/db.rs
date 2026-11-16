//! SQLite 持久化层。
//!
//! 结构化数据（项目、设备会话、执行记录、AI 对话）落库于 `modulink.db`。
//! 大附件与日志以文件形式存储，数据库只记录引用。
//!
//! 迁移系统：`schema_migrations` 表记录已应用版本，启动时按版本号顺序补齐缺失迁移，
//! 每个迁移在一个事务内执行，失败即回滚并阻止启动。
use crate::app_paths::AppPaths;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 数据库句柄，作为 Tauri State 注入。Connection 非同步，用 Mutex 包裹。
#[derive(Clone)]
pub struct Db(pub Arc<Mutex<Connection>>);

impl Db {
    /// 打开（或创建）数据库并应用迁移。
    pub fn open(paths: &AppPaths) -> Result<Self, String> {
        let db_file = paths.db_file();
        if let Some(parent) = db_file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建数据库目录失败: {e}"))?;
        }
        let mut conn = Connection::open(&db_file)
            .map_err(|e| format!("打开数据库失败 ({}): {e}", db_file.display()))?;
        // WAL 提升读写并发；普通 DELETE 模式下写会阻塞读。
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| format!("设置 WAL 失败: {e}"))?;
        // 外键约束默认关闭，需显式开启。
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| format!("开启外键约束失败: {e}"))?;
        // 写入繁忙时等待，减少 SQLITE_BUSY。
        conn.pragma_update(None, "busy_timeout", 5000)
            .map_err(|e| format!("设置 busy_timeout 失败: {e}"))?;
        migrate(&mut conn)?;
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    /// 在锁保护下执行闭包。闭包返回 rusqlite::Result，由外层统一转为 String。
    fn with_conn<R>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<R>,
    ) -> Result<R, String> {
        let conn = self.0.lock();
        f(&conn).map_err(|e| format!("数据库操作失败: {e}"))
    }
}

// ---------------------------------------------------------------------------
// 迁移系统
// ---------------------------------------------------------------------------

/// 一个迁移：版本号 + SQL（多条语句）+ 可选描述。
struct Migration {
    version: u32,
    description: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    description:
        "Phase A: projects / device_sessions / executions / ai_conversations / ai_messages",
    sql: r#"
CREATE TABLE IF NOT EXISTS projects (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    customer        TEXT,
    device_model    TEXT,
    firmware        TEXT,
    sim_env         TEXT,
    notes           TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS device_sessions (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    port            TEXT NOT NULL,
    baud            INTEGER,
    profile_id      TEXT,
    identified_at   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS executions (
    id              TEXT PRIMARY KEY,
    session_id      TEXT REFERENCES device_sessions(id) ON DELETE SET NULL,
    project_id      TEXT REFERENCES projects(id) ON DELETE SET NULL,
    source          TEXT NOT NULL,
    kind            TEXT NOT NULL,
    status          TEXT NOT NULL,
    started_at      INTEGER NOT NULL,
    finished_at     INTEGER NOT NULL,
    elapsed_ms      INTEGER NOT NULL,
    step_count      INTEGER NOT NULL DEFAULT 0,
    port            TEXT,
    summary_json    TEXT,
    results_json    TEXT,
    results_truncated INTEGER NOT NULL DEFAULT 0,
    error           TEXT,
    created_at      INTEGER NOT NULL DEFAULT (strftime('%s','now') * 1000)
);
CREATE INDEX IF NOT EXISTS idx_executions_started_at ON executions(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_executions_session ON executions(session_id);

CREATE TABLE IF NOT EXISTS ai_conversations (
    id              TEXT PRIMARY KEY,
    session_id      TEXT REFERENCES device_sessions(id) ON DELETE SET NULL,
    project_id      TEXT REFERENCES projects(id) ON DELETE SET NULL,
    created_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS ai_messages (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES ai_conversations(id) ON DELETE CASCADE,
    role            TEXT NOT NULL,
    content_json    TEXT NOT NULL,
    created_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ai_messages_conversation ON ai_messages(conversation_id, created_at);
"#,
}];

fn migrate(conn: &mut Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version    INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL DEFAULT (strftime('%s','now') * 1000),
            description TEXT
        );",
    )
    .map_err(|e| format!("创建 schema_migrations 表失败: {e}"))?;

    let applied: Vec<u32> = {
        let mut stmt = conn
            .prepare("SELECT version FROM schema_migrations ORDER BY version")
            .map_err(|e| format!("查询迁移记录失败: {e}"))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, u32>(0))
            .map_err(|e| format!("读取迁移记录失败: {e}"))?;
        rows.filter_map(Result::ok).collect()
    };

    for m in MIGRATIONS {
        if applied.contains(&m.version) {
            continue;
        }
        let tx = conn
            .transaction()
            .map_err(|e| format!("开启迁移事务 v{} 失败: {e}", m.version))?;
        tx.execute_batch(m.sql)
            .map_err(|e| format!("迁移 v{} 执行失败: {e}", m.version))?;
        tx.execute(
            "INSERT INTO schema_migrations (version, description) VALUES (?1, ?2)",
            params![m.version, m.description],
        )
        .map_err(|e| format!("记录迁移 v{} 失败: {e}", m.version))?;
        tx.commit()
            .map_err(|e| format!("提交迁移 v{} 失败: {e}", m.version))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 领域 DTO（与前端 types 对齐，camelCase 序列化）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub customer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sim_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 执行记录的完整快照（含 results/summary 的 JSON）。落库时由后端构造。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRecord {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub source: String,
    pub kind: String,
    pub status: String,
    pub started_at: i64,
    pub finished_at: i64,
    pub elapsed_ms: u64,
    pub step_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub results: Option<serde_json::Value>,
    #[serde(default)]
    pub results_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// 数据访问
// ---------------------------------------------------------------------------

impl Db {
    /// 保存（upsert）执行记录。失败不应影响主执行流程，调用方负责隔离错误。
    pub fn save_execution(&self, rec: &ExecutionRecord) -> Result<(), String> {
        let summary = rec
            .summary
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| format!("序列化 summary 失败: {e}"))?;
        let results = rec
            .results
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| format!("序列化 results 失败: {e}"))?;
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO executions (id, session_id, project_id, source, kind, status,
                 started_at, finished_at, elapsed_ms, step_count, port, summary_json,
                 results_json, results_truncated, error)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
                 ON CONFLICT(id) DO UPDATE SET
                   session_id=excluded.session_id, project_id=excluded.project_id,
                   status=excluded.status, finished_at=excluded.finished_at,
                   elapsed_ms=excluded.elapsed_ms, summary_json=excluded.summary_json,
                   results_json=excluded.results_json, results_truncated=excluded.results_truncated,
                   error=excluded.error",
                params![
                    rec.id,
                    rec.session_id,
                    rec.project_id,
                    rec.source,
                    rec.kind,
                    rec.status,
                    rec.started_at,
                    rec.finished_at,
                    rec.elapsed_ms as i64,
                    rec.step_count as i64,
                    rec.port,
                    summary,
                    results,
                    rec.results_truncated as i64,
                    rec.error,
                ],
            )
        })
        .map(|_| ())
    }

    /// 按开始时间倒序查询最近执行记录。limit 为条数上限。
    pub fn list_executions(&self, limit: u32) -> Result<Vec<ExecutionRecord>, String> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT id, session_id, project_id, source, kind, status, started_at,
                 finished_at, elapsed_ms, step_count, port, summary_json, results_json,
                 results_truncated, error
                 FROM executions ORDER BY started_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit as i64], row_to_execution)?;
            rows.collect()
        })
    }

    pub fn create_project(&self, p: &Project) -> Result<(), String> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO projects (id, name, customer, device_model, firmware, sim_env,
                 notes, created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    p.id,
                    p.name,
                    p.customer,
                    p.device_model,
                    p.firmware,
                    p.sim_env,
                    p.notes,
                    p.created_at,
                    p.updated_at,
                ],
            )
        })
        .map(|_| ())
    }

    pub fn list_projects(&self) -> Result<Vec<Project>, String> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT id, name, customer, device_model, firmware, sim_env, notes,
                 created_at, updated_at FROM projects ORDER BY updated_at DESC",
            )?;
            let rows = stmt.query_map([], row_to_project)?;
            rows.collect()
        })
    }

    pub fn get_project(&self, id: &str) -> Result<Option<Project>, String> {
        self.with_conn(|c| {
            c.query_row(
                "SELECT id, name, customer, device_model, firmware, sim_env, notes,
                 created_at, updated_at FROM projects WHERE id = ?1",
                params![id],
                row_to_project,
            )
            .optional()
        })
    }
}

fn row_to_execution(r: &rusqlite::Row) -> rusqlite::Result<ExecutionRecord> {
    let summary_str: Option<String> = r.get("summary_json")?;
    let results_str: Option<String> = r.get("results_json")?;
    let summary = summary_str
        .as_deref()
        .map(serde_json::from_str::<serde_json::Value>)
        .transpose()
        .unwrap_or(None);
    let results = results_str
        .as_deref()
        .map(serde_json::from_str::<serde_json::Value>)
        .transpose()
        .unwrap_or(None);
    Ok(ExecutionRecord {
        id: r.get("id")?,
        session_id: r.get("session_id")?,
        project_id: r.get("project_id")?,
        source: r.get("source")?,
        kind: r.get("kind")?,
        status: r.get("status")?,
        started_at: r.get("started_at")?,
        finished_at: r.get("finished_at")?,
        elapsed_ms: r.get::<_, i64>("elapsed_ms")? as u64,
        step_count: r.get::<_, i64>("step_count")? as usize,
        port: r.get("port")?,
        summary,
        results,
        results_truncated: r.get::<_, i64>("results_truncated")? != 0,
        error: r.get("error")?,
    })
}

fn row_to_project(r: &rusqlite::Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: r.get("id")?,
        name: r.get("name")?,
        customer: r.get("customer")?,
        device_model: r.get("device_model")?,
        firmware: r.get("firmware")?,
        sim_env: r.get("sim_env")?,
        notes: r.get("notes")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
    })
}

/// 生成主键标识。时间毫秒 + 进程内计数，主键唯一性约束兜底。
pub fn new_id() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let ms = chrono::Utc::now().timestamp_millis();
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{}-{}-{}", ms, n / 65536, n % 65536)
}
