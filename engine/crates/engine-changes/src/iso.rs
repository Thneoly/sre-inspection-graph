//! ISO8601 时间工具(复刻 reference `event_service._now_iso`/`_shift_iso` +
//! `frequency._parse_iso`/`alert_correlation._parse_iso_local`,统一去重)。
//!
//! 全部走 `YYYY-MM-DDTHH:MM:SSZ` 固定格式 -- 同格式同区下字符串字典序 == 时间序,
//! [`crate::models::ChangeFilter`] 的 `since`/`until` 闭区间过滤依赖此性质。

#![allow(missing_docs)]

use chrono::{DateTime, Duration, Utc};

/// 当前 UTC 时间 ISO8601 `YYYY-MM-DDTHH:MM:SSZ`(对齐 reference `_now_iso`)。
pub fn now_iso() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// 宽松解析 ISO8601(尾 `Z` 或带偏移;无 tz 视作 UTC)。失败返 `None`
/// (对齐 reference `_parse_iso` / `_parse_iso_local`)。
pub fn parse_iso_utc(s: &str) -> Option<DateTime<Utc>> {
    if s.is_empty() {
        return None;
    }
    let s = s.trim();
    let dt = if let Some(stripped) = s.strip_suffix('Z') {
        DateTime::parse_from_rfc3339(&format!("{}+00:00", stripped)).ok()?
    } else {
        DateTime::parse_from_rfc3339(s).ok()?
    };
    Some(dt.with_timezone(&Utc))
}

/// 把 ISO8601 字符串前后平移 N 秒,保持 `Z` 后缀(对齐 reference `_shift_iso`)。
///
/// 解析失败时原样返回(调用方对无效时间戳宽容处理)。
pub fn shift_iso(iso: &str, delta_seconds: i64) -> String {
    match parse_iso_utc(iso) {
        Some(dt) => {
            let shifted = dt + Duration::seconds(delta_seconds);
            shifted.format("%Y-%m-%dT%H:%M:%SZ").to_string()
        }
        None => iso.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn now_iso_is_z_suffix_fixed_width() {
        let s = now_iso();
        assert!(s.ends_with('Z'));
        assert_eq!(s.len(), "2026-07-10T03:00:00Z".len());
    }

    #[test]
    fn parse_z_and_offset() {
        assert_eq!(
            parse_iso_utc("2026-07-10T03:00:00Z"),
            Some(Utc.with_ymd_and_hms(2026, 7, 10, 3, 0, 0).unwrap())
        );
        assert_eq!(
            parse_iso_utc("2026-07-10T03:00:00+00:00"),
            Some(Utc.with_ymd_and_hms(2026, 7, 10, 3, 0, 0).unwrap())
        );
        assert!(parse_iso_utc("").is_none());
        assert!(parse_iso_utc("not-a-date").is_none());
    }

    #[test]
    fn shift_iso_adds_and_subtracts() {
        assert_eq!(shift_iso("2026-07-10T03:00:00Z", 300), "2026-07-10T03:05:00Z");
        assert_eq!(shift_iso("2026-07-10T03:05:00Z", -300), "2026-07-10T03:00:00Z");
        // 跨日
        assert_eq!(shift_iso("2026-07-10T23:59:00Z", 120), "2026-07-11T00:01:00Z");
        // 解析失败原样返回
        assert_eq!(shift_iso("garbage", 300), "garbage");
    }
}
