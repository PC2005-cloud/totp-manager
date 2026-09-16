//! 应用入口与前后端桥梁。
//!
//! 这个文件做三件事：
//! 1. 定义**前端能看到的数据结构**（`EntryView`）
//! 2. 定义**前端能调用的命令**（标了 `#[tauri::command]` 的函数）
//! 3. 启动 Tauri 并把命令注册进去（`run()`）
//!
//! # 前后端怎么通信
//! 前端 JavaScript 用 `invoke("命令名", { 参数 })` 发起调用，
//! 参数名必须和后端函数的参数名**完全一致**（Tauri 靠名字匹配）。
//! 后端返回 `Result`：`Ok` 变成 JS 的 resolve，`Err` 变成 reject。
//!
//! # 举个例子
//! ```javascript
//! // 前端这样调
//! const entries = await invoke("list_entries");
//! await invoke("add_entry", { label: "GitHub", account: "me@x.com", secret: "JBSW..." });
//! ```
//!
//! 对应的后端函数就是下面的 `list_entries` 和 `add_entry`。

mod totp;
mod vault;

use std::time::{SystemTime, UNIX_EPOCH};
use vault::Entry;

/// 发给前端展示的一条记录。
///
/// 比 `Entry` 多了三个「现算出来」的字段：`code`、`remaining`、`error`。
/// 这三个不进文件，只是每次查询时根据当前时间临时算好一起发给前端，
/// 省得前端自己算（前端算不了，算法在 Rust 这边）。
#[derive(serde::Serialize)]
struct EntryView {
    /// 显示名，如「GitHub 工作」
    label: String,

    /// 账号，如邮箱
    account: String,

    /// 原始 Base32 密钥，前端用来在编辑时回填
    secret: Option<String>,

    /// 恢复码列表，前端直接展示
    codes: Option<Vec<String>>,

    /// 备注
    note: Option<String>,

    /// 当前 6 位验证码。密钥缺失或非法时为 `None`
    code: Option<String>,

    /// 当前验证码剩余有效秒数（1~30），前端用它显示倒计时
    remaining: u64,

    /// 密钥有问题时的错误说明，前端可以直接显示
    error: Option<String>,
}

/// 取当前 Unix 时间戳（秒）。
///
/// 单独抽出来是为了统一处理「系统时间早于 1970 年」这种异常情况 ——
/// 那种情况下 `duration_since` 会报错，这里退化成返回 0 而不是崩溃。
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 把存储用的 `Entry` 转成发给前端的 `EntryView`。
///
/// 主要工作是算验证码。注意三种情况的处理：
/// - 有密钥且合法 → 算出验证码
/// - 有密钥但非法 → `code` 为空，`error` 说明原因
/// - 压根没有密钥（纯恢复码记录）→ 两个都是空，不算错误
fn to_view(e: Entry) -> EntryView {
    let t = now();

    let (code, error) = match e.secret.as_deref() {
        Some(s) if !s.trim().is_empty() => match totp::code(s, t) {
            Ok(c) => (Some(c), None),
            Err(msg) => (None, Some(msg)),
        },
        _ => (None, None),
    };

    EntryView {
        label: e.label,
        account: e.account,
        secret: e.secret,
        codes: e.codes,
        note: e.note,
        code,
        remaining: totp::remaining_seconds(t),
        error,
    }
}

/// 【前端命令】列出全部记录，每条附带当前验证码。
///
/// 前端每秒调用一次以刷新验证码和倒计时。
/// 记录放在内存里没有缓存，每次都从文件重新读 —— 几十条记录的开销可以忽略，
/// 换来的是「用外部编辑器改了文件也能立刻看到」。
///
/// # 返回
/// - `Ok(记录列表)`：可能为空数组。
/// - `Err(...)`：数据文件读不了或格式坏了。
#[tauri::command]
fn list_entries() -> Result<Vec<EntryView>, String> {
    Ok(vault::load()?.into_iter().map(to_view).collect())
}

/// 【前端命令】新增一条记录。
///
/// 会先校验再写入：
/// - `label` 和 `account` 不能是空白（这是必填字段）
/// - 密钥和备注如果传了空字符串，会被当成「没填」而存成 `None`
///
/// 新增的记录不带恢复码（`codes` 为 `None`），
/// 目前版本还没有编辑恢复码的界面。
///
/// # 参数（名字必须和前端 invoke 时一致）
/// - `label`：显示名
/// - `account`：账号
/// - `secret`：Base32 密钥，可以不传
/// - `note`：备注，可以不传
///
/// # 返回
/// - `Ok(())`：写入成功。
/// - `Err(...)`：校验不通过，或写文件失败。
#[tauri::command]
fn add_entry(
    label: String,
    account: String,
    secret: Option<String>,
    note: Option<String>,
) -> Result<(), String> {
    if label.trim().is_empty() {
        return Err("名称不能为空".into());
    }
    if account.trim().is_empty() {
        return Err("账号不能为空".into());
    }

    let mut entries = vault::load()?;
    entries.push(Entry {
        label: label.trim().to_string(),
        account: account.trim().to_string(),
        // 空字符串视作「没填」，转成 None 让 JSON 更干净
        secret: secret.filter(|s| !s.trim().is_empty()),
        codes: None,
        note: note.filter(|s| !s.trim().is_empty()),
    });
    vault::save(&entries)
}

/// 【前端命令】删除一条记录。
///
/// 靠 `label` + `account` 两个字段共同定位要删的那条 ——
/// 因为同一个服务可能有多个账号，只用 `label` 会误删。
///
/// 如果没找到匹配的记录，函数仍然返回 `Ok(())`（删除操作是幂等的）。
///
/// # 参数
/// - `label`：要删的记录名
/// - `account`：要删的记录账号
#[tauri::command]
fn delete_entry(label: String, account: String) -> Result<(), String> {
    let mut entries = vault::load()?;
    entries.retain(|e| !(e.label == label && e.account == account));
    vault::save(&entries)
}

/// 【前端命令】返回数据文件的完整路径。
///
/// 界面上显示这个路径，方便用户自己备份或手动编辑文件。
#[tauri::command]
fn vault_location() -> Result<String, String> {
    Ok(vault::vault_path()?.display().to_string())
}

/// 启动应用 —— 由 `main.rs` 调用。
///
/// 这里只做两件事：注册上面那四个命令，然后进入 Tauri 的事件循环。
/// 事件循环会一直阻塞到用户关闭窗口为止。
///
/// `#[cfg_attr(mobile, ...)]` 是为了将来移植到移动端时能自动生成入口点，
/// 在桌面端没有作用。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            list_entries,
            add_entry,
            delete_entry,
            vault_location
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}
