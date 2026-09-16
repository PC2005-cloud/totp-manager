//! TOTP 验证码算法。
//!
//! 这个文件负责把「Base32 密钥 + 当前时间」算成「6 位数字验证码」。
//! 遵循 RFC 6238（TOTP）与 RFC 4226（HOTP）标准。
//!
//! 核心公式：
//! ```text
//! counter = 当前Unix时间戳 / 30          （每 30 秒加 1）
//! 摘要     = HMAC-SHA1(密钥, counter的8字节大端表示)
//! 验证码   = 从摘要里「动态截断」出 4 字节整数，再对 1000000 取模
//! ```
//!
//! 这就是 Google Authenticator、微软 Authenticator 等 App 用的同一套算法。
//! 只要密钥相同、时钟一致，本程序和手机 App 会算出完全一样的数字。

use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;

/// HMAC-SHA1 的类型别名。
/// `Hmac<Sha1>` 表示「用 SHA1 作为哈希函数的 HMAC」，写起来太长所以起个别名。
type HmacSha1 = Hmac<Sha1>;

/// 时间步长（秒）。
///
/// 验证码每 30 秒变化一次，这是 RFC 6238 推荐值，也是绝大多数服务的取值。
const PERIOD: u64 = 30;

/// 验证码位数。
///
/// 标准是 6 位。少数服务用 8 位，本项目暂不支持（见 DESIGN.md 的决策记录）。
const DIGITS: u32 = 6;

/// 计算 TOTP 验证码 —— 本模块对外的唯一入口。
///
/// # 参数
/// - `secret`：Base32 编码的密钥字符串。可以带空格、连字符、`=` 填充、小写字母，
///   函数内部会自动清理，所以从网页上直接复制粘贴通常都能用。
/// - `unix_time`：Unix 时间戳（秒）。传当前时间就得到当前验证码。
///
/// # 返回
/// - `Ok("123456")`：6 位验证码，**已补足前导零**（所以是字符串而不是数字）。
/// - `Err(...)`：密钥为空或不是合法的 Base32，错误信息是中文，可直接显示给用户。
///
/// # 例子
/// ```text
/// // RFC 6238 官方测试用例：密钥 "12345678901234567890"，t=59 时应得 287082
/// code("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", 59)  // => Ok("287082")
/// ```
pub fn code(secret: &str, unix_time: u64) -> Result<String, String> {
    let key = decode_secret(secret)?;
    Ok(code_from_key(&key, unix_time))
}

/// 距离当前验证码失效还剩多少秒 —— 用于界面上显示倒计时。
///
/// 返回值范围是 1 到 30：
/// - 刚跨入新周期时返回 30（还有整整 30 秒可用）
/// - 即将失效时返回 1
///
/// # 例子
/// ```text
/// remaining_seconds(0)   // => 30  周期刚开始
/// remaining_seconds(1)   // => 29
/// remaining_seconds(29)  // => 1   马上要变了
/// remaining_seconds(30)  // => 30  进入下一个周期
/// ```
pub fn remaining_seconds(unix_time: u64) -> u64 {
    PERIOD - (unix_time % PERIOD)
}

/// 把用户输入的 Base32 密钥清理干净，再解码成字节数组。
///
/// 清理内容包括：去掉所有空白字符、连字符 `-`、填充符 `=`，并统一转成大写。
/// 这样用户在网页上看到的 `abcd efgh-ijkl` 这类格式都能直接用。
///
/// # 返回
/// - `Ok(字节数组)`：解码成功，长度通常是 10、20 或 32 字节。
/// - `Err("密钥为空")`：清理后什么都不剩。
/// - `Err("密钥不是合法的 Base32: ...")`：含有 Base32 字符集之外的字符。
fn decode_secret(secret: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = secret
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '=')
        .flat_map(|c| c.to_uppercase())
        .collect();

    if cleaned.is_empty() {
        return Err("密钥为空".into());
    }

    data_encoding::BASE32_NOPAD
        .decode(cleaned.as_bytes())
        .map_err(|e| format!("密钥不是合法的 Base32: {e}"))
}

/// 核心算法：用已解码的密钥字节算验证码。
///
/// 与 `code()` 的区别是这个函数收的是**原始字节**，不做 Base32 解码，
/// 因此可以被测试代码用 RFC 官方测试向量直接调用。
///
/// 实现分三步，对应 RFC 4226 第 5.3 节的 HOTP 算法：
/// 1. 算出计数器，做 HMAC-SHA1 得到 20 字节摘要
/// 2. **动态截断**：用摘要最后一字节的低 4 位当偏移量，从该位置取 4 字节
/// 3. 抹掉最高位（避免当成负数），对 10^6 取模得到 6 位数字
fn code_from_key(key: &[u8], unix_time: u64) -> String {
    // 第 1 步：时间转计数器，做 HMAC-SHA1。
    // 计数器必须是 8 字节大端整数，这是 RFC 4226 的硬性规定。
    let counter = unix_time / PERIOD;

    let mut mac = HmacSha1::new_from_slice(key).expect("HMAC 接受任意长度密钥");
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();

    // 第 2 步：动态截断。
    // 取摘要最后一个字节（第 20 字节，下标 19）的低 4 位，得到 0~15 的偏移量。
    // 用哈希自身决定取哪几个字节，能避免固定位置取样带来的统计偏置。
    let offset = (digest[19] & 0x0f) as usize;
    let bin = u32::from_be_bytes([
        digest[offset],
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]) & 0x7fff_ffff; // 抹掉符号位，保证是正数

    // 第 3 步：取模得到 6 位数字。
    // 用 {:06} 补前导零 —— 少了这一步，「1234」会被显示成 4 位数而不是 001234。
    let modulus = 10u32.pow(DIGITS);
    format!("{:0width$}", bin % modulus, width = DIGITS as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用 RFC 6238 附录 B 的官方测试向量验证算法正确性。
    ///
    /// 官方给的是 8 位验证码，本项目用 6 位，所以比对的是后 6 位。
    /// 这 6 组数据覆盖了不同年代的时间戳（1970 年代到 2603 年），
    /// 能确认算法在极端时间值下也正确。
    #[test]
    fn rfc6238_vectors() {
        let key = b"12345678901234567890";
        let cases: [(u64, &str); 6] = [
            (59, "287082"),
            (1111111109, "081804"),
            (1111111111, "050471"),
            (1234567890, "005924"),
            (2000000000, "279037"),
            (20000000000, "353130"),
        ];
        for (t, want) in cases {
            assert_eq!(code_from_key(key, t), want, "t={t}");
        }
    }

    /// 验证各种「脏」密钥输入都能被正确清理并解析。
    ///
    /// 现实中用户从网页复制密钥，常带空格、连字符、填充符或小写字母，
    /// 这个测试确保这些格式都不会导致失败。
    #[test]
    fn base32_roundtrip_and_formatting() {
        // 标准 Base32（无填充）
        assert_eq!(code("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", 59).unwrap(), "287082");
        // 带空格、连字符、小写、填充，都应能正常解析
        assert_eq!(
            code("gezd gnbv-gy3t qojq gezd gnbv gy3t qojq====", 59).unwrap(),
            "287082"
        );
    }

    /// 验证非法密钥会被拒绝，而不是算出错误的验证码。
    ///
    /// 这点很重要：静默算出一个错的码会让用户以为是账号问题，
    /// 不如直接报错让用户知道密钥填错了。
    #[test]
    fn rejects_bad_secret() {
        assert!(code("", 59).is_err());
        assert!(code("!!!!", 59).is_err());
    }

    /// 验证倒计时计算的边界情况。
    ///
    /// 重点是周期性：t=30 必须回到 30，而不是 0 或负数。
    #[test]
    fn remaining_seconds_counts_down() {
        assert_eq!(remaining_seconds(0), 30);
        assert_eq!(remaining_seconds(1), 29);
        assert_eq!(remaining_seconds(29), 1);
        assert_eq!(remaining_seconds(30), 30);
    }
}
