//! 程序入口。
//!
//! 这个文件刻意保持极简 —— 真正的逻辑都在 `lib.rs` 里。
//! 这是 Tauri 项目的标准做法：把代码放在 lib 里，main 只负责调用，
//! 好处是 lib 可以被测试代码和将来可能的其他前端（如移动端）复用。

// Windows 发布版不弹控制台窗口。
// debug 构建时保留控制台，方便看 println! 和 panic 信息。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// 调用 lib 里的 `run()`，进入 Tauri 事件循环。
///
/// 这个函数不会正常返回 —— 它会一直运行到用户关闭窗口。
fn main() {
    totp_manager_lib::run()
}
