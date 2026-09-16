//! 数据存取层。
//!
//! 负责把记录读写成 JSON 文件。文件位置固定在 **exe 所在目录**下：
//! ```text
//! <exe 所在目录>\vault.json
//! ```
//! - 开发时：`src-tauri\target\debug\vault.json`
//! - 发布后把 exe 放进软件目录：`D:\Software\TOTPManager\vault.json`
//!
//! 设计目标是「**一个 exe + 一个 json**」：程序拷到哪，数据就跟到哪，
//! 卸载时删掉整个文件夹即可，不往系统目录（如 `%APPDATA%`）里留任何东西。
//!
//! # 文件格式
//! 顶层直接是一个数组，每项是一条记录，**没有外层包装对象，也没有版本号**：
//! ```json
//! [
//!   {
//!     "label": "GitHub 工作",
//!     "account": "me@company.com",
//!     "secret": "JBSWY3DPEHPK3PXP",
//!     "codes": ["a1b2c3d4e5", "f6g7h8i9j0"],
//!     "note": "备用"
//!   }
//! ]
//! ```
//!
//! # 安全说明
//! 这个文件是**明文存储**，`secret` 字段可以直接被任何程序读走。
//! 设计上接受这个取舍（见 DESIGN.md），但有两条纪律：
//! 不要把数据文件放进云同步目录；确保它被写在 `.gitignore` 里。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// 一条记录：一个账号对应一个 TOTP 密钥，可选若干恢复码。
///
/// `label` 和 `account` 是必填的，其余三个字段都可以没有。
/// 标了 `#[serde(default)]` 的字段在 JSON 里缺失时会自动读成 `None`，
/// 所以文件里可以只写用得上的字段。
#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    /// 显示名，例如「GitHub 工作」。必填。
    pub label: String,

    /// 账号，例如邮箱或用户名。必填。
    ///
    /// 同一个服务常有多个账号（工作号 / 个人号），`label` 可能重名，
    /// `account` 才是真正区分它们的东西。
    pub account: String,

    /// TOTP 密钥（Base32 字符串）。没有密钥的记录为 `None`。
    ///
    /// 存的是**原始字符串**而非解码后的字节，这样能原样导出、
    /// 出问题时也能肉眼核对。
    #[serde(default)]
    pub secret: Option<String>,

    /// 恢复码列表。没存恢复码的记录为 `None`。
    ///
    /// 恢复码是一次性的：用掉就从数组里删掉。
    #[serde(default)]
    pub codes: Option<Vec<String>>,

    /// 自由备注。没写备注的记录为 `None`。
    #[serde(default)]
    pub note: Option<String>,
}

/// 返回数据文件的完整路径：**exe 同目录**下的 `vault.json`。
///
/// 取的是可执行文件自身的所在目录，而不是当前工作目录 ——
/// 因为工作目录会变（双击启动、从命令行启动、快捷方式启动都不一样），
/// 而 exe 的位置是固定的。
///
/// # 效果
/// - 开发时：`src-tauri\target\debug\vault.json`
/// - 发布后：把 `totp-manager.exe` 拷到 `D:\Software\TOTPManager\`，
///   数据就是 `D:\Software\TOTPManager\vault.json`，跟 exe 待在一起
///
/// 这样软件整体可移动：整个文件夹拷到 U 盘、换台电脑，都能直接继续用。
///
/// # 返回
/// - `Ok(路径)`：正常情况下总是成功。
/// - `Err(...)`：极端情况下取不到程序自身路径（极少见）。
pub fn vault_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("找不到程序位置: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| "找不到程序所在目录".to_string())?;
    Ok(dir.join("vault.json"))
}

/// 读取全部记录。
///
/// 这个函数被设计成**友好地处理「还没有数据」的情况**，
/// 让首次使用的用户不会看到任何报错：
/// - 文件不存在 → 返回空列表
/// - 文件存在但是空的 → 返回空列表
///
/// # 返回
/// - `Ok(记录列表)`：可能为空。顺序就是文件里的顺序。
/// - `Err(...)`：文件存在但读不了，或 JSON 格式坏了。
pub fn load() -> Result<Vec<Entry>, String> {
    let path = vault_path()?;

    // 首次运行：文件还不存在，视作空列表而不是错误
    if !path.exists() {
        return Ok(Vec::new());
    }

    let text = fs::read_to_string(&path).map_err(|e| format!("读取失败: {e}"))?;

    // 空文件（比如用户手动清空了内容）同样视作空列表
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }

    serde_json::from_str(&text).map_err(|e| format!("解析失败: {e}"))
}

/// 覆盖写入全部记录。
///
/// 整个文件会被重写，所以调用方必须先 `load()`、改完、再 `save()`。
/// 记录数量很少（几十条），全量重写的开销可以忽略。
///
/// # 返回
/// - `Ok(())`：写入成功。
/// - `Err(...)`：创建目录失败、序列化失败或写盘失败。
pub fn save(entries: &[Entry]) -> Result<(), String> {
    let path = vault_path()?;

    // 项目根目录正常都存在，这里只是兜底（比如 exe 被拷到别处运行）
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("创建目录失败: {e}"))?;
    }

    // 用 pretty 格式输出，方便用户自己打开文件查看和手动编辑
    let text = serde_json::to_string_pretty(entries).map_err(|e| format!("序列化失败: {e}"))?;
    fs::write(&path, text).map_err(|e| format!("写入失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证 JSON 解析能同时处理「字段齐全」和「只有必填字段」两种记录。
    ///
    /// 这直接对应真实使用：有的账号既有密钥又有恢复码，
    /// 有的只有密钥，两者都必须能正常读出来。
    #[test]
    fn entry_json_shape() {
        // 完整记录
        let full = r#"[
          {
            "label": "GitHub 工作",
            "account": "me@company.com",
            "secret": "JBSWY3DPEHPK3PXP",
            "codes": ["a1b2c3d4e5", "f6g7h8i9j0"],
            "note": "备用"
          }
        ]"#;
        let v: Vec<Entry> = serde_json::from_str(full).unwrap();
        assert_eq!(v[0].label, "GitHub 工作");
        assert_eq!(v[0].codes.as_ref().unwrap().len(), 2);

        // 省略可选字段也应能解析 —— 验证 #[serde(default)] 生效
        let minimal = r#"[{ "label": "论坛", "account": "myname", "secret": "KRSXG5CTMVRXEZLU" }]"#;
        let v: Vec<Entry> = serde_json::from_str(minimal).unwrap();
        assert!(v[0].codes.is_none());
        assert!(v[0].note.is_none());
    }

    /// 验证「序列化再反序列化」不会丢数据或改变内容。
    ///
    /// 这是存取层最关键的性质：写进文件再读回来必须一模一样。
    #[test]
    fn roundtrip_through_json() {
        let entries = vec![Entry {
            label: "GitHub".into(),
            account: "me@example.com".into(),
            secret: Some("JBSWY3DPEHPK3PXP".into()),
            codes: Some(vec!["aaa".into()]),
            note: None,
        }];
        let text = serde_json::to_string(&entries).unwrap();
        let back: Vec<Entry> = serde_json::from_str(&text).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].secret.as_deref(), Some("JBSWY3DPEHPK3PXP"));
        assert!(back[0].note.is_none());
    }

    /// 验证数据文件确实和 exe 放在同一个目录。
    ///
    /// 这是本项目「一个 exe + 一个 json」设计的关键性质：
    /// 发布时把 exe 拷到软件目录，数据文件必须跟着在同一层，
    /// 而不是跑到 C 盘用户目录或项目根目录去。
    #[test]
    fn vault_path_is_next_to_exe() {
        let path = vault_path().unwrap();

        // 文件名必须是 vault.json
        assert_eq!(path.file_name().unwrap(), "vault.json");

        // 所在目录必须和 exe 所在目录完全一致
        let exe_dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
        assert_eq!(
            path.parent().unwrap(),
            exe_dir,
            "vault.json 应位于 exe 同目录，实际是: {}",
            path.display()
        );
    }
}
