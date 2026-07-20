//! 报告订阅模型 + 内存 registry + cron 解析(PRD-003 Phase 4.3,对齐 reference
//! `subscription_store.py` + `scheduler.py` 的 cron 解析)。
//!
//! 全 I/O-free 纯逻辑:SubscriptionStore 是内存 registry(持久化在 engine-storage,
//! orchestration 层 load/upsert)。cron 用 `cron` crate 解析 5-field crontab
//! (prepend "0 " 秒位 -> 6-field)。

#![allow(missing_docs)]

use std::collections::HashMap;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use cron::Schedule;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::ReportScope;
use crate::ReportTemplate;

/// 订阅最近一次执行状态(对齐 reference `last_status`:never | ok | failed)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionStatus {
    /// 从未执行。
    #[default]
    Never,
    /// 最近一次成功。
    Ok,
    /// 最近一次失败。
    Failed,
}

/// 报告订阅(对齐 reference `ReportSubscription` dataclass)。
///
/// 调度器按 `cron` 触发,用 `scope` + `modules` 构造 `ReportTask`,生成后发邮件给
/// `recipients`,回写 `last_*` 字段。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportSubscription {
    /// `sub-<12 hex>`。
    pub subscription_id: String,
    /// 报告模板。
    pub template_id: ReportTemplate,
    /// 范围(application_id / cluster_id / change_event_id / fault_id / time_range)。
    pub scope: ReportScope,
    /// 启用的模块子集(空 = 全模块)。
    pub modules: Vec<String>,
    /// 5-field crontab(例 `0 9 * * 1` = 每周一 9:00)。
    pub cron: String,
    /// 收件人邮箱列表(非空)。
    pub recipients: Vec<String>,
    /// 是否启用(禁用则调度器跳过)。
    pub enabled: bool,
    /// 创建时间(ISO8601)。
    pub created_at: String,
    /// 最近一次执行时间(ISO8601;空 = 从未)。
    pub last_run_at: String,
    /// 最近一次执行状态。
    pub last_status: SubscriptionStatus,
    /// 最近一次执行错误(空 = 无)。
    pub last_error: String,
    /// 最近一次生成的报告 ID(空 = 无)。
    pub last_report_id: String,
}

impl ReportSubscription {
    /// 生成新订阅 ID(`sub-<12 hex>`,对齐 reference `new_subscription_id`)。
    pub fn new_id() -> String {
        format!("sub-{}", Uuid::new_v4().simple())
    }

    /// `last_run_at` 解析为 DateTime;空或非法 -> `created_at` 解析 -> 都失败 -> None。
    pub fn last_run_dt(&self) -> Option<DateTime<Utc>> {
        if !self.last_run_at.is_empty() {
            if let Some(dt) = parse_iso(&self.last_run_at) {
                return Some(dt);
            }
        }
        parse_iso(&self.created_at)
    }
}

/// 校验订阅字段(template_id 合法 / cron 可解析 / recipients 非空)。
/// 返回 Err(message) 供 command 层 400。
pub fn validate_subscription(
    template_id: ReportTemplate,
    cron: &str,
    recipients: &[String],
) -> Result<(), String> {
    let _ = parse_cron(cron)?;
    if recipients.is_empty() {
        return Err("recipients 不能为空".to_string());
    }
    // template_id 已是强类型枚举,无需再校验
    let _ = template_id;
    Ok(())
}

/// 解析 5-field crontab(`min hour dom mon dow`)为 `cron::Schedule`。
///
/// `cron` crate 用 6-7 field(`sec min hour dom mon dow [year]`),故 prepend `"0 "`
/// 秒位。非法 -> Err(message)。
pub fn parse_cron(cron_5field: &str) -> Result<Schedule, String> {
    let six_field = format!("0 {cron_5field}");
    Schedule::from_str(&six_field).map_err(|e| format!("invalid cron '{cron_5field}': {e}"))
}

fn parse_iso(s: &str) -> Option<DateTime<Utc>> {
    if s.is_empty() {
        return None;
    }
    let s = s.trim();
    if let Some(stripped) = s.strip_suffix('Z') {
        DateTime::parse_from_rfc3339(&format!("{stripped}+00:00"))
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    } else {
        DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    }
}

/// 订阅内存 registry(对齐 reference `SubscriptionStore`;持久化在 orchestration 层)。
#[derive(Debug, Clone, Default)]
pub struct SubscriptionStore {
    subs: HashMap<String, ReportSubscription>,
}

impl SubscriptionStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 从已加载订阅列表构造(orchestration 从 storage 恢复用)。
    pub fn from_subscriptions(subs: Vec<ReportSubscription>) -> Self {
        Self {
            subs: subs.into_iter().map(|s| (s.subscription_id.clone(), s)).collect(),
        }
    }

    pub fn add(&mut self, sub: ReportSubscription) {
        self.subs.insert(sub.subscription_id.clone(), sub);
    }

    pub fn get(&self, id: &str) -> Option<&ReportSubscription> {
        self.subs.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut ReportSubscription> {
        self.subs.get_mut(id)
    }

    pub fn delete(&mut self, id: &str) -> bool {
        self.subs.remove(id).is_some()
    }

    /// 列表(新到旧,按 created_at 降序;可按 template_id 过滤)。
    pub fn list(&self, template_id: Option<ReportTemplate>) -> Vec<&ReportSubscription> {
        let mut v: Vec<&ReportSubscription> = self
            .subs
            .values()
            .filter(|s| template_id.is_none_or(|t| s.template_id == t))
            .collect();
        v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        v
    }

    pub fn clear(&mut self) {
        self.subs.clear();
    }

    pub fn len(&self) -> usize {
        self.subs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.subs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(id: &str, tpl: ReportTemplate, cron: &str) -> ReportSubscription {
        ReportSubscription {
            subscription_id: id.into(),
            template_id: tpl,
            scope: ReportScope::default(),
            modules: vec![],
            cron: cron.into(),
            recipients: vec!["ops@example.com".into()],
            enabled: true,
            created_at: format!("2026-07-20T0{id}:00:00Z"),
            last_run_at: String::new(),
            last_status: SubscriptionStatus::Never,
            last_error: String::new(),
            last_report_id: String::new(),
        }
    }

    #[test]
    fn parse_cron_valid_5field() {
        assert!(parse_cron("0 9 * * 1").is_ok()); // 每周一 9:00
        assert!(parse_cron("*/5 * * * *").is_ok()); // 每 5 分钟
        assert!(parse_cron("0 0 * * *").is_ok()); // 每日 0:00
    }

    #[test]
    fn parse_cron_invalid() {
        assert!(parse_cron("not a cron").is_err());
        assert!(parse_cron("99 * * * *").is_err()); // 分钟越界
        assert!(parse_cron("* * *").is_err()); // 字段不足
    }

    #[test]
    fn validate_subscription_rejects_empty_recipients() {
        let err = validate_subscription(ReportTemplate::ApplicationHealth, "0 9 * * 1", &[]);
        assert!(err.is_err());
        assert!(validate_subscription(ReportTemplate::ApplicationHealth, "0 9 * * 1", &["a@b.c".into()]).is_ok());
    }

    #[test]
    fn validate_subscription_rejects_bad_cron() {
        let err = validate_subscription(ReportTemplate::ApplicationHealth, "bad", &["a@b.c".into()]);
        assert!(err.is_err());
    }

    #[test]
    fn new_id_format() {
        let id = ReportSubscription::new_id();
        assert!(id.starts_with("sub-"));
        assert_eq!(id.len(), "sub-".len() + 32); // Uuid::simple() = 32 hex(对齐 rpt-{simple})
    }

    #[test]
    fn store_crud_and_list_order() {
        let mut store = SubscriptionStore::new();
        store.add(sub("sub-b", ReportTemplate::ApplicationHealth, "0 9 * * 1"));
        store.add(sub("sub-a", ReportTemplate::ClusterOverview, "0 9 * * 1"));
        // created_at: sub-b="...01:00", sub-a="...00:00" -> 倒序 sub-b 在前
        let listed = store.list(None);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].subscription_id, "sub-b");
        // template 过滤
        assert_eq!(store.list(Some(ReportTemplate::ClusterOverview)).len(), 1);
        assert!(store.get("sub-a").is_some());
        assert!(store.get_mut("sub-a").is_some());
        assert!(store.delete("sub-a"));
        assert!(store.get("sub-a").is_none());
        assert!(!store.delete("sub-a"));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn from_subscriptions_constructs() {
        let store = SubscriptionStore::from_subscriptions(vec![
            sub("sub-1", ReportTemplate::ApplicationHealth, "0 9 * * 1"),
            sub("sub-2", ReportTemplate::ApplicationHealth, "0 9 * * 1"),
        ]);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn last_run_dt_falls_back_to_created_at() {
        let mut s = sub("sub-x", ReportTemplate::ApplicationHealth, "0 9 * * 1");
        s.created_at = "2026-07-20T09:00:00Z".into();
        // last_run_at 空 -> 回退 created_at
        assert_eq!(s.last_run_dt(), parse_iso("2026-07-20T09:00:00Z"));
        s.last_run_at = "2026-07-21T09:00:00Z".into();
        assert_eq!(s.last_run_dt(), parse_iso("2026-07-21T09:00:00Z"));
    }
}
