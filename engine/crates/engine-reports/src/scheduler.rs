//! 调度触发判定(PRD-003 Phase 4.3,对齐 reference `_run_subscription_safely` 触发时机)。
//!
//! 纯函数:给定 `Schedule` + `last_run` + `now` + `grace`,判定该不该触发。调度循环
//! (orchestration 层 / desktop)每 tick 调此函数。无 I/O、无时钟依赖(`now` 由调用方传)。
//!
//! **no-catch-up 语义**(对齐 reference `misfire_grace_time=300` + `coalesce=True`):
//! - 下次触发 > now -> 未到期,跳过。
//! - 下次触发 <= now 且 now - next <= grace -> 触发(关机 < grace 重开补跑 1 次)。
//! - 下次触发 <= now 且 now - next > grace -> 漏发超 grace,跳过但推进 last_run_at 到
//!   next_fire(消费 stale fire,防止反复评估;下次评估算 next_fire 之后的触发)。

#![allow(missing_docs)]

use chrono::{DateTime, Duration, Utc};
use cron::Schedule;

/// 触发判定结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FireDecision {
    /// 触发:跑 generate + email,然后 `last_run_at = now`。
    Fire,
    /// 未到期(下次触发 > now),不触发,`last_run_at` 不变。
    NotDue,
    /// 漏发超 grace(关机期间错过),跳过本次,但 `last_run_at` 应推进到 `next_fire`
    /// 防止反复评估同一 stale fire。
    MissedAdvance(DateTime<Utc>),
}

/// 默认 grace 5 分钟(对齐 reference `misfire_grace_time=300`)。
pub const DEFAULT_GRACE_SECS: i64 = 300;

/// 默认 grace(`Duration::seconds(DEFAULT_GRACE_SECS)`)。
pub fn default_grace() -> Duration {
    Duration::seconds(DEFAULT_GRACE_SECS)
}

/// 判定订阅是否该在 `now` 触发。
///
/// `last_run_at` = 最近一次执行时间(从未执行则传订阅 `created_at`)。
/// `schedule.after(last_run_at).next()` = last_run_at 之后的第一个 cron 触发点。
pub fn check_fire(
    schedule: &Schedule,
    last_run_at: DateTime<Utc>,
    now: DateTime<Utc>,
    grace: Duration,
) -> FireDecision {
    let next_fire = match schedule.after(&last_run_at).next() {
        Some(t) => t,
        None => return FireDecision::NotDue,
    };
    if next_fire > now {
        return FireDecision::NotDue;
    }
    // next_fire <= now:有错过的触发点
    if (now - next_fire) <= grace {
        FireDecision::Fire
    } else {
        FireDecision::MissedAdvance(next_fire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;

    fn dt(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    // "0 9 * * *" = 每日 09:00 UTC(parse_cron prepend "0 " -> "0 0 9 * * *")
    fn daily_9am() -> Schedule {
        crate::parse_cron("0 9 * * *").unwrap()
    }

    #[test]
    fn not_due_when_next_fire_after_now() {
        // last_run 08:00, now 08:30 -> next_fire 09:00 > now -> NotDue
        let s = daily_9am();
        let d = check_fire(&s, dt("2026-07-20T08:00:00Z"), dt("2026-07-20T08:30:00Z"), default_grace());
        assert_eq!(d, FireDecision::NotDue);
    }

    #[test]
    fn fire_when_within_grace() {
        // last_run 08:00, now 09:00:30 -> next_fire 09:00:00, now-next=30s <= 300s -> Fire
        let s = daily_9am();
        let d = check_fire(&s, dt("2026-07-20T08:00:00Z"), dt("2026-07-20T09:00:30Z"), default_grace());
        assert_eq!(d, FireDecision::Fire);
    }

    #[test]
    fn missed_advance_when_beyond_grace() {
        // last_run 2026-07-19 08:00, now 2026-07-20 10:00 -> next_fire 2026-07-19 09:00
        // (first after last_run), now-next=25h > 300s -> MissedAdvance(2026-07-19 09:00)
        let s = daily_9am();
        let d = check_fire(&s, dt("2026-07-19T08:00:00Z"), dt("2026-07-20T10:00:00Z"), default_grace());
        let expected_next = dt("2026-07-19T09:00:00Z");
        assert_eq!(d, FireDecision::MissedAdvance(expected_next));
    }

    #[test]
    fn fire_exactly_at_fire_time() {
        // last_run 08:00, now 09:00:00 (== next_fire) -> now-next=0 <= grace -> Fire
        let s = daily_9am();
        let d = check_fire(&s, dt("2026-07-20T08:00:00Z"), dt("2026-07-20T09:00:00Z"), default_grace());
        assert_eq!(d, FireDecision::Fire);
    }

    #[test]
    fn never_run_uses_provided_last_run() {
        // 调用方对"从未执行"传 created_at;last_run=created=07-20 08:00, now 09:00:30 -> Fire
        let s = daily_9am();
        let d = check_fire(&s, dt("2026-07-20T08:00:00Z"), dt("2026-07-20T09:00:30Z"), default_grace());
        assert_eq!(d, FireDecision::Fire);
    }
}
