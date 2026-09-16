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
/// - 密钥、恢复码、备注如果传了空值，会被当成「没填」而存成 `None`
///
/// # 参数（名字必须和前端 invoke 时一致）
/// - `label`：服务名，如 `GitHub`
/// - `account`：账号
/// - `secret`：Base32 密钥，可以不传
/// - `codes`：恢复码列表，可以不传
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
    codes: Option<Vec<String>>,
    note: Option<String>,
) -> Result<(), String> {
    if label.trim().is_empty() {
        return Err("服务不能为空".into());
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
        codes: clean_codes(codes),
        note: note.filter(|s| !s.trim().is_empty()),
    });
    vault::save(&entries)
}

/// 【前端命令】覆盖某条记录的恢复码列表。
///
/// 传入的列表会**整体替换**原有内容，而不是追加。删除单条恢复码时，
/// 前端把「剩下的那些」整个传过来即可，不需要单独写一个删除命令。
///
/// 传空列表或 `None` 会把该记录的恢复码清空。
///
/// # 参数
/// - `label` / `account`：定位要修改的记录
/// - `codes`：新的恢复码列表
#[tauri::command]
fn update_recovery_codes(
    label: String,
    account: String,
    codes: Option<Vec<String>>,
) -> Result<(), String> {
    let mut entries = vault::load()?;
    let target = entries
        .iter_mut()
        .find(|e| e.label == label && e.account == account)
        .ok_or_else(|| format!("找不到记录：{label} · {account}"))?;

    target.codes = clean_codes(codes);
    vault::save(&entries)
}

/// 【前端命令】修改一条已有记录。
///
/// 用**原服务名 + 原账号**定位要改的那条，然后把整条记录替换成新内容。
/// 之所以要分开传原值，是因为服务名和账号本身也可能被改 ——
/// 如果只用新值定位，改名后就找不到原记录了。
///
/// 会校验两件事：
/// - 新的服务名和账号都不能为空
/// - 改完之后不能和另一条记录撞车（同服务 + 同账号）
///
/// # 参数
/// - `original_label` / `original_account`：改之前的值，用来定位
/// - `entry`：改之后的内容
#[tauri::command]
fn update_entry(
    original_label: String,
    original_account: String,
    entry: Entry,
) -> Result<(), String> {
    let new_label = entry.label.trim().to_string();
    let new_account = entry.account.trim().to_string();

    if new_label.is_empty() {
        return Err("服务不能为空".into());
    }
    if new_account.is_empty() {
        return Err("账号不能为空".into());
    }

    let mut entries = vault::load()?;

    // 改名后不能和别的记录重复，否则删除时会一次删掉两条
    let clash = entries.iter().any(|e| {
        e.label == new_label
            && e.account == new_account
            && !(e.label == original_label && e.account == original_account)
    });
    if clash {
        return Err(format!("已存在「{new_label} · {new_account}」"));
    }

    let target = entries
        .iter_mut()
        .find(|e| e.label == original_label && e.account == original_account)
        .ok_or_else(|| format!("找不到记录：{original_label} · {original_account}"))?;

    *target = Entry {
        label: new_label,
        account: new_account,
        secret: entry.secret.filter(|s| !s.trim().is_empty()),
        codes: clean_codes(entry.codes),
        note: entry.note.filter(|s| !s.trim().is_empty()),
    };

    vault::save(&entries)
}

/// 清理恢复码列表：逐条去空白、丢掉空串，全空则返回 `None`。
///
/// 这样 JSON 里不会出现 `"codes": []` 或 `"codes": ["", " "]` 这类噪音，
/// 「没有恢复码」始终表示为字段缺失。
fn clean_codes(codes: Option<Vec<String>>) -> Option<Vec<String>> {
    let cleaned: Vec<String> = codes
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect();

    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
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

/* ------------------------------------------------------------------------- */
/* 窗口控制                                                                   */
/* ------------------------------------------------------------------------- */
/* Windows 原生标题栏没法改样式，所以 tauri.conf.json 里关掉了 `decorations`， */
/* 标题栏由前端自绘（index.html 的 <header> + style.css 的 .win-*）。          */
/* 下面这几个命令就是自绘标题栏需要的能力。                                    */
/*                                                                           */
/* 为什么不用 `data-tauri-drag-region` 属性：它内部走的是 Tauri 核心窗口命令，  */
/* 需要在 capabilities 文件里显式授权；本项目至今没有任何 capabilities 文件，   */
/* 而**自己定义的命令不受 ACL 限制**，所以直接自己写更省事，也和现有架构一致。 */
/* ------------------------------------------------------------------------- */

/// 【前端命令】开始拖动窗口 —— 自绘标题栏上按下鼠标时调用。
///
/// 由系统接管后续的拖动，所以这个调用本身很快就返回，窗口跟着鼠标走。
#[tauri::command]
fn window_start_drag(window: tauri::WebviewWindow) -> Result<(), String> {
    window.start_dragging().map_err(|e| format!("拖动窗口失败: {e}"))
}

/// 【前端命令】最小化窗口。
#[tauri::command]
fn window_minimize(window: tauri::WebviewWindow) -> Result<(), String> {
    window.minimize().map_err(|e| format!("最小化失败: {e}"))
}

/// 【前端命令】最大化 / 还原 切换，返回切换之后**是否处于最大化**。
///
/// 返回新状态是为了让前端立刻换成对的图标（最大化 ↔ 还原）。
/// 判断和切换都以后端的实际窗口状态为准，所以即使前端的状态记错了
/// （比如用户是按 Win+↑ 最大化的），点一下也会自动纠正回来。
#[tauri::command]
fn window_toggle_maximize(window: tauri::WebviewWindow) -> Result<bool, String> {
    let maximized = window
        .is_maximized()
        .map_err(|e| format!("读取窗口状态失败: {e}"))?;

    if maximized {
        window.unmaximize().map_err(|e| format!("还原失败: {e}"))?;
    } else {
        window.maximize().map_err(|e| format!("最大化失败: {e}"))?;
    }

    Ok(!maximized)
}

/// 【前端命令】窗口当前是否最大化。
///
/// 前端在窗口尺寸变化后调用它来同步图标 —— 用户可能用 Win+↑、拖动窗口
/// 或者双击标题栏改变最大化状态，前端的记录会过时。
#[tauri::command]
fn window_is_maximized(window: tauri::WebviewWindow) -> Result<bool, String> {
    window
        .is_maximized()
        .map_err(|e| format!("读取窗口状态失败: {e}"))
}

/// 【前端命令】关闭窗口（同时也就退出了程序）。
#[tauri::command]
fn window_close(window: tauri::WebviewWindow) -> Result<(), String> {
    window.close().map_err(|e| format!("关闭窗口失败: {e}"))
}

/// 启动应用 —— 由 `main.rs` 调用。
///
/// 这里只做两件事：注册上面那些命令，然后进入 Tauri 的事件循环。
/// 事件循环会一直阻塞到用户关闭窗口为止。
///
/// `#[cfg_attr(mobile, ...)]` 是为了将来移植到移动端时能自动生成入口点，
/// 在桌面端没有作用。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            // 数据相关
            list_entries,
            add_entry,
            update_entry,
            delete_entry,
            update_recovery_codes,
            vault_location,
            // 自绘标题栏相关
            window_start_drag,
            window_minimize,
            window_toggle_maximize,
            window_is_maximized,
            window_close
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}
