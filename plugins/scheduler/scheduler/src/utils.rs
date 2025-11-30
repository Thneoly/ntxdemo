use anyhow::{Context, Result};
use std::time::Duration;

/// 解析时间字符串为 Duration
///
/// 支持的格式：
/// - "500ms" - 毫秒
/// - "1s" - 秒
/// - "1m" - 分钟
///
/// # Examples
/// ```
/// let d = parse_duration("1s").unwrap();
/// assert_eq!(d, Duration::from_secs(1));
///
/// let d = parse_duration("500ms").unwrap();
/// assert_eq!(d, Duration::from_millis(500));
/// ```
pub fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim();

    if let Some(ms_str) = s.strip_suffix("ms") {
        let ms: u64 = ms_str
            .trim()
            .parse()
            .with_context(|| format!("Invalid milliseconds value: {}", ms_str))?;
        Ok(Duration::from_millis(ms))
    } else if let Some(s_str) = s.strip_suffix('s') {
        let secs: u64 = s_str
            .trim()
            .parse()
            .with_context(|| format!("Invalid seconds value: {}", s_str))?;
        Ok(Duration::from_secs(secs))
    } else if let Some(m_str) = s.strip_suffix('m') {
        let mins: u64 = m_str
            .trim()
            .parse()
            .with_context(|| format!("Invalid minutes value: {}", m_str))?;
        Ok(Duration::from_secs(mins * 60))
    } else {
        anyhow::bail!(
            "Invalid duration format: '{}'. Expected format: <number>ms|s|m",
            s
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_milliseconds() {
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(
            parse_duration("1000ms").unwrap(),
            Duration::from_millis(1000)
        );
        assert_eq!(parse_duration("1ms").unwrap(), Duration::from_millis(1));
    }

    #[test]
    fn test_parse_seconds() {
        assert_eq!(parse_duration("1s").unwrap(), Duration::from_secs(1));
        assert_eq!(parse_duration("10s").unwrap(), Duration::from_secs(10));
        assert_eq!(parse_duration("0s").unwrap(), Duration::from_secs(0));
    }

    #[test]
    fn test_parse_minutes() {
        assert_eq!(parse_duration("1m").unwrap(), Duration::from_secs(60));
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
    }

    #[test]
    fn test_parse_with_whitespace() {
        assert_eq!(parse_duration(" 1s ").unwrap(), Duration::from_secs(1));
        assert_eq!(
            parse_duration("500 ms").unwrap(),
            Duration::from_millis(500)
        );
    }

    #[test]
    fn test_parse_invalid() {
        assert!(parse_duration("invalid").is_err());
        assert!(parse_duration("1").is_err());
        assert!(parse_duration("s").is_err());
    }
}
