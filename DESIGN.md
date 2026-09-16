# totp-manager 数据模型设计

本地 2FA 管理工具。明文 JSON 存储，不加密、不联网、不做同步。

## 存储位置与格式

- 单个 JSON 文件，**顶层直接是数组**，无外层包装对象、无版本号
- 存储位置：**与 exe 同目录**下的 `vault.json`
  - 开发时：`src-tauri\target\debug\vault.json`
  - 发布后：`D:\Software\TOTPManager\vault.json`（exe 拷到哪，数据就在哪）
- 取 **exe 自身所在目录**，不用当前工作目录 —— 工作目录会因启动方式（双击 / 命令行 / 快捷方式）而变，exe 位置固定
- 设计目标「**一个 exe + 一个 json**」：整体可移动，拷到 U 盘或换台电脑都能继续用；卸载时删掉文件夹即可，不往 `%APPDATA%` 等系统目录留东西
- 注意：不要放进云同步目录；`.gitignore` 中已排除 `vault.json`

```json
[
  {
    "label": "GitHub 工作",
    "account": "me@company.com",
    "secret": "JBSWY3DPEHPK3PXP",
    "codes": ["a1b2c3d4e5", "f6g7h8i9j0"]
  },
  {
    "label": "某论坛",
    "account": "myname",
    "secret": "KRSXG5CTMVRXEZLU"
  }
]
```

## 字段定义

| 字段 | 类型 | 可空 | 说明 |
|---|---|---|---|
| `label` | String | 否 | 显示名，如 `GitHub 工作` |
| `account` | String | 否 | 账号，如邮箱 / 用户名 |
| `secret` | Option\<String\> | 是 | TOTP 密钥，Base32 原始字符串 |
| `codes` | Option\<Vec\<String\>\> | 是 | 恢复码列表 |
| `note` | Option\<String\> | 是 | 备注 |

可空字段共三个：`secret` / `codes` / `note`。

### Rust 定义

```rust
#[derive(Serialize, Deserialize)]
struct Entry {
    label: String,
    account: String,
    #[serde(default)]
    secret: Option<String>,
    #[serde(default)]
    codes: Option<Vec<String>>,
    #[serde(default)]
    note: Option<String>,
}
```

`#[serde(default)]` 使字段缺失时读为 `None`，不报错。

## 设计决策记录

**顶层直接是数组** —— 文件里只有一种东西（记录），无需 `entries` 包装层，也无需 `version` 字段。个人自用工具，改结构时直接改文件，不需要程序做旧格式兼容判断。

**`label` 与 `account` 均必填，且 `label` 在前** —— 同一个服务常有多账号（工作号 / 个人号），`label` 会重名，`account` 才是实质区分标识。二者都填才能唯一定位一条记录。

**`account` 不可空** —— 所有目标服务都绑定在账号上（邮箱 / 手机号 / 用户名）。即便界面只要求一个名字，该名字本身即是账号；实在没有独立账号时，与 `label` 填相同值即可。

**`secret` 存原始 Base32 字符串，不存解码后的字节** —— 便于原样导出为 `otpauth://` URI，出问题时肉眼可核对，且不依赖 Base32 实现正确性。仅在计算验证码时解码。

**`secret` 与 `codes` 同属一条记录（合并方案）** —— 一个账号一条记录，点进去同时看到动态码与恢复码，避免列表中出现两条同名记录。两者独立可空，因为并非每个账号都同时具备：有密钥无恢复码、仅有恢复码（如银行）都常见。

**不存储 `algorithm` / `digits` / `period`** —— 99% 的服务使用默认值（SHA1 / 6 位 / 30 秒），为极少见情况给全部记录增加字段不划算。这三个参数在代码中硬编码为常量：

```rust
const ALGORITHM: Algo = Algo::Sha1;
const DIGITS: u32 = 6;
const PERIOD: u64 = 30;
```

若日后确实导入到非默认服务、算出的码对不上，再添加对应可选字段；读取时缺省走默认值，向后兼容自动成立。

**恢复码不做 `used` 标记** —— 用掉即删除。个人工具无需追踪历史用量；日后确有需要，加一个字段即可。

**不存的字段** —— 账户密码（密码管理器的职责）、`use_count` / `last_used_at`、标签 / 分组 / 收藏、图标 / 颜色、同步相关字段。

## 计算参数

| 参数 | 值 |
|---|---|
| 算法 | HMAC-SHA1 |
| 位数 | 6 |
| 时间步长 | 30 秒 |
| 编码 | Base32（无填充） |

## 框架选型：Tauri 2

桌面 GUI 采用 **Tauri 2**，非 Electron、非 egui。

**理由** —— 体积小（复用系统 WebView2，产物数 MB 而非上百 MB）、内存占用低、原生托盘支持、Rust 侧可直接调用 Windows API。个人小工具无需 Electron 的运行时代价，也无需 egui 自己实现整套 UI。

**环境实测**（`cargo tauri info` 全绿）：

| 组件 | 版本 / 状态 |
|---|---|
| Tauri CLI | 2.11.4 |
| WebView2 Runtime | 153.0.4234.32 |
| MSVC | Visual Studio Community 2026 |
| rustc / cargo | 1.97.1 |
| Node / pnpm / npm | 24.19.0 / 11.22.0 / 11.17.0 |

前端工具链齐备，无需额外安装。安装包打包（WiX / NSIS）暂不需要，因此 `rust-lld`、WiX、NSIS 均未安装，不影响开发运行。

## 依赖

**Rust 侧**（`src-tauri/Cargo.toml`）：

```toml
[dependencies]
hmac = "0.13"
sha1 = "0.11"
data-encoding = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
time = "0.3"
tauri = { version = "2", features = [] }
```

**版本验证结论** —— `hmac 0.13` + `sha1 0.11` 属 RustCrypto 新一代（基于 `digest 0.11`），API 与旧版 0.12/0.10 的差异经实测确认**不影响本用法**：

```rust
let mut mac = HmacSha1::new_from_slice(secret)?;  // KeyInit
mac.update(&counter.to_be_bytes());              // Mac
let digest = mac.finalize().into_bytes();        // 20 字节
```

已用 RFC 6238 附录 B 的 6 组官方测试向量实测，**6/6 全部通过**，Base32 往返亦正确。

**前端侧**（`package.json`）—— 保持最小，初期可只用原生 HTML/JS，不引入框架：

```json
{
  "devDependencies": { "@tauri-apps/cli": "^2" }
}
```

若后续界面复杂化再引入框架，但简单列表 + 倒计时用原生实现足够。

## 项目结构

Tauri 要求前端与 Rust 分离，目录如下：

```
totp-manager/
├── DESIGN.md
├── package.json              # 前端工具链
├── index.html                # 界面入口
├── src/                      # 前端 JS/CSS
│   ├── main.js
│   └── style.css
└── src-tauri/
    ├── Cargo.toml
    ├── tauri.conf.json       # 窗口、打包配置
    ├── build.rs
    ├── icons/
    └── src/
        ├── main.rs           # Tauri 入口，注册命令
        ├── totp.rs           # HMAC-SHA1 + 动态截断
        └── vault.rs          # JSON 数组读写
```

**注意**：现有 `src/main.rs`（`Hello, world!`）需迁移到 `src-tauri/src/main.rs`，根目录 `src/` 改作前端代码。根 `Cargo.toml` 相应移除或改为 workspace 成员。

## 待办

- [ ] 迁移项目结构至 Tauri 布局（`src-tauri/` + 前端 `src/`）
- [ ] `totp.rs` —— HMAC-SHA1 + 动态截断（算法已验证，落地即可）
- [ ] `vault.rs` —— JSON 数组读写
- [ ] Tauri 命令：`list_entries` / `add_entry` / `delete_entry` / `get_code`
- [ ] 前端：列表 + 实时倒计时 + 一键复制
- [ ] 剪贴板自动清空（复制验证码后 30 秒）
