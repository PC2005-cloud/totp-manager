# totp-manager

本地 TOTP（两步验证）管理器 —— **一个 exe + 一个 JSON**，不联网、不加密、不同步。

基于 Tauri 2 + Rust，界面是原生 HTML/CSS/JS（没有前端框架），Windows 上复用系统 WebView2，体积和内存都只是一个原生小工具的量级。

## 功能

- **动态验证码** —— HMAC-SHA1 / 6 位 / 30 秒（RFC 6238），与 Google Authenticator、Microsoft Authenticator 算出的码一致
- **恢复码** —— 与密钥存在同一条记录里，一个账号一条，点进去同时看到动态码和恢复码；用掉即删除
- **服务分组** —— 按服务名分组、标题吸顶、可折叠，折叠状态记在本地
- **搜索** —— 按服务名 / 账号 / 备注实时过滤
- **一键复制** —— 点验证码即复制（复制的是去掉空格的原始码），恢复码逐条复制
- **删除二次确认** —— 记录和恢复码都要 3 秒内点两次才真的删，防手滑
- **亮暗主题** —— 跟随系统或手动切换，选择记在本地
- **自绘标题栏** —— 无边框窗口 + 自己的标题栏（拖动、双击最大化、最小化 / 最大化 / 关闭）
- **可以手动改数据文件** —— 程序每秒重新读一次 `vault.json`，用外部编辑器改完立刻生效

## 数据存储

数据就是 exe 同目录下的一个 `vault.json`，**顶层直接是数组**：

```json
[
  {
    "label": "GitHub 工作",
    "account": "me@company.com",
    "secret": "JBSWY3DPEHPK3PXP",
    "codes": ["a1b2c3d4e5", "f6g7h8i9j0"],
    "note": "备用"
  }
]
```

| 字段 | 类型 | 可空 | 说明 |
|---|---|---|---|
| `label` | String | 否 | 服务名 |
| `account` | String | 否 | 账号 |
| `secret` | String | 是 | TOTP 密钥（Base32 原始字符串） |
| `codes` | String[] | 是 | 恢复码列表 |
| `note` | String | 是 | 备注 |

设计目标是「一个 exe + 一个 json」：整体可移动，拷到 U 盘或换台电脑都能接着用；卸载时删掉文件夹即可，不往 `%APPDATA%` 之类的系统目录写东西。开发时数据文件在 `src-tauri/target/debug/vault.json`，发布后与 exe 同目录。

> ⚠️ **明文存储**：`vault.json` 里的密钥不做任何加密，任何程序都能读走。请不要把它放进云同步目录（OneDrive / Dropbox 等），并确保它被 `.gitignore` 排除（本仓库已排除）。

## 构建与运行

需要 Rust（edition 2024，1.85+）和 Node + pnpm（用于 Tauri CLI）：

```bash
pnpm install
pnpm dev                              # 开发运行
pnpm build                            # 构建 exe（暂未启用安装包打包）
```

也可以只用 cargo：

```bash
cargo run --manifest-path src-tauri/Cargo.toml
```

## 测试

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

TOTP 算法用 RFC 6238 附录 B 的 6 组官方测试向量校验；存储层另有 JSON 解析、往返、BOM 容忍等用例。

## 项目结构

```
totp-manager/
├── DESIGN.md              # 数据模型与设计决策记录
├── src/                   # 前端（原生 HTML/CSS/JS）
│   ├── index.html
│   ├── style.css          # 玻璃风格 + 亮暗主题变量
│   └── main.js
└── src-tauri/
    ├── Cargo.toml
    ├── tauri.conf.json    # 窗口（无边框）、打包配置
    └── src/
        ├── main.rs        # 入口
        ├── lib.rs         # Tauri 命令（数据存取 + 窗口控制）
        ├── totp.rs        # HMAC-SHA1 + 动态截断
        └── vault.rs       # vault.json 读写
```

## 待办

- [ ] 复制验证码后 30 秒自动清空剪贴板

## 文档

数据模型和取舍理由（为什么明文、为什么顶层是数组、为什么不存 `algorithm` / `digits` / `period`）见 [DESIGN.md](DESIGN.md)。
