//! 8 种 RecoveryAction 模板定义 + 影响传播规则 + 推荐映射。
//!
//! 复刻 `reference/app/recovery/action_defs.py`(read-only oracle)。这是 recovery
//! 运行时的 single source of truth:`ActionDef` 元数据 + `propagation` 规则(给
//! [`crate::cascade::dry_run`] 用)+ rule/change -> action 推荐。
//!
//! ## 与 reference 的差异
//!
//! - 8 个动作为命名 `static`(便于 `ActionSuggestion.action: &'static ActionDef` 在
//!   static 上下文引用),`ACTION_DEFS` 是 `&[&'static ActionDef]`;`ActionDef` /
//!   `PropagationRule` / `ParamSpec` 均 `Copy`。
//! - `input_schema` 用 typed `&[ParamSpec]` 而非 JSON schema dict(3.2 参数校验时用)。
//! - 元数据字段(action_id / risk_level / requires_approval / rollback_action_id /
//!   propagation)与 reference 逐字对齐,contract test 钉死。

#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

/// 动作风险等级。low = 同步执行无需审批;medium/high = 需审批(Phase 3 桌面单机确认门)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// 同步执行,无需审批。
    Low,
    /// 需审批,影响面单实例。
    Medium,
    /// 需审批,影响面跨实例。
    High,
}

/// 受影响节点的严重度等级(多规则命中同一节点取较大值)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Impact {
    /// 几乎无感。
    Minimal,
    /// 轻微影响。
    Low,
    /// 中等影响。
    Medium,
    /// 严重影响。
    High,
}

impl Impact {
    /// 严重度序:越大越严重。用于多规则命中取 max + 排序。
    pub fn rank(self) -> u8 {
        match self {
            Impact::Minimal => 0,
            Impact::Low => 1,
            Impact::Medium => 2,
            Impact::High => 3,
        }
    }
}

/// 传播遍历方向。forward = 顺着 edge(source->target);reverse = 逆着(target->source)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// 顺着关系:edge.source == current -> next = edge.target。
    Forward,
    /// 逆着关系:edge.target == current -> next = edge.source。
    Reverse,
}

/// 动作输入参数的类型(3.2 参数校验用)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    /// 布尔。
    Boolean,
    /// 整数。
    Integer,
    /// 字符串(含 enum,3.2 再校验取值)。
    String,
}

/// 一个输入参数的规格。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ParamSpec {
    /// 参数名。
    pub name: &'static str,
    /// 参数类型。
    pub kind: ParamKind,
    /// 是否必填。
    pub required: bool,
}

/// 一条影响传播规则(给 [`crate::cascade::dry_run`] BFS 用)。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PropagationRule {
    /// 关系类型(CONTAINS / USES / ROUTES_TO / BELONGS_TO / SCHEDULED_ON / DEPLOYED_AS)。
    pub edge: &'static str,
    /// 遍历方向。
    pub direction: Direction,
    /// 遍历深度(避免深递归)。
    pub max_depth: u8,
    /// 筛选目标节点类型(None = 不筛)。
    pub target_type: Option<&'static str>,
    /// 受影响节点的严重度。
    pub impact: Impact,
    /// 人读影响描述。
    pub note: &'static str,
}

/// 一个 RecoveryAction 模板(8 个之一)。
///
/// 只 Serialize(给 Tauri 返回用),不 Deserialize -- 含 `&'static` 字段(static 构造),
/// 引用无法反序列化。3.2 execute 管线按 `action_id: String` 查 [`get_action`],不反序列化 ActionDef。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ActionDef {
    /// 动作 ID。
    pub action_id: &'static str,
    /// 人读名称(中文)。
    pub name: &'static str,
    /// 分类(availability / scale / rollback / config / drain / other)。
    pub category: &'static str,
    /// 目标资源类型(Pod / Deployment / Secret / KubernetesNode / MySQL / Redis / Service)。
    pub target_type: &'static str,
    /// 风险等级。
    pub risk_level: RiskLevel,
    /// 是否需审批(Phase 3 桌面单机确认门:medium/high 一般 true;kill_query 例外)。
    pub requires_approval: bool,
    /// 回滚动作 ID(None = 不可逆;Some(self) = 再跑一次)。
    pub rollback_action_id: Option<&'static str>,
    /// 预估耗时(秒)。
    pub estimated_duration_seconds: u32,
    /// 人读描述。
    pub description: &'static str,
    /// 输入参数规格。
    pub input_schema: &'static [ParamSpec],
    /// 影响传播规则集。
    pub propagation: &'static [PropagationRule],
    /// 预估 SLA 影响。
    pub sla_impact_estimate: &'static str,
    /// 警告提示。
    pub warnings: &'static [&'static str],
}

// ===== 8 个命名 static(逐字对齐 reference ACTION_DEFS)=====

pub static RESTART_POD: ActionDef = ActionDef {
    action_id: "restart_pod",
    name: "重启 Pod",
    category: "availability",
    target_type: "Pod",
    risk_level: RiskLevel::Medium,
    requires_approval: true,
    rollback_action_id: None,
    estimated_duration_seconds: 60,
    description: "对目标 Pod 执行 kubectl delete pod,触发 ReplicaSet 自动重新调度。",
    input_schema: &[
        ParamSpec { name: "graceful", kind: ParamKind::Boolean, required: false },
        ParamSpec { name: "grace_period_seconds", kind: ParamKind::Integer, required: false },
    ],
    propagation: &[
        PropagationRule { edge: "ROUTES_TO", direction: Direction::Reverse, max_depth: 1, target_type: Some("Service"), impact: Impact::Low, note: "Service Endpoints 临时少 1 个就绪 Pod" },
        PropagationRule { edge: "CONTAINS", direction: Direction::Reverse, max_depth: 1, target_type: Some("Deployment"), impact: Impact::Minimal, note: "ReplicaSet 自动重新调度新 Pod" },
        PropagationRule { edge: "BELONGS_TO", direction: Direction::Forward, max_depth: 3, target_type: None, impact: Impact::Minimal, note: "向上影响 Component / Application(短暂感知)" },
    ],
    sla_impact_estimate: "< 0.1%",
    warnings: &[
        "该 Pod 提供的服务在 30-60 秒内不可用",
        "若 Pod 是 Deployment 唯一副本(replicas=1),将引发短暂服务中断",
    ],
};

pub static SCALE_DEPLOYMENT: ActionDef = ActionDef {
    action_id: "scale_deployment",
    name: "调整 Deployment 副本",
    category: "scale",
    target_type: "Deployment",
    risk_level: RiskLevel::Low,
    requires_approval: false,
    rollback_action_id: Some("scale_deployment"),
    estimated_duration_seconds: 90,
    description: "对 Deployment 增减副本数。正数扩容,负数缩容。",
    input_schema: &[
        ParamSpec { name: "replicas_delta", kind: ParamKind::Integer, required: true },
    ],
    propagation: &[
        PropagationRule { edge: "CONTAINS", direction: Direction::Forward, max_depth: 1, target_type: Some("Pod"), impact: Impact::Minimal, note: "新增/减少 Pod 副本" },
        PropagationRule { edge: "BELONGS_TO", direction: Direction::Forward, max_depth: 2, target_type: None, impact: Impact::Minimal, note: "Component 承载能力变化" },
    ],
    sla_impact_estimate: "< 0.1%",
    warnings: &[
        "扩容后成本会增加,建议业务低峰期再缩容",
        "缩容到 < 期望副本数会触发 Pod 删除,影响在 Pod 上的连接",
    ],
};

pub static ROLLBACK_DEPLOYMENT: ActionDef = ActionDef {
    action_id: "rollback_deployment",
    name: "回滚 Deployment 版本",
    category: "rollback",
    target_type: "Deployment",
    risk_level: RiskLevel::High,
    requires_approval: true,
    rollback_action_id: Some("rollback_deployment"),
    estimated_duration_seconds: 180,
    description: "kubectl rollout undo,把 Deployment 回退到上一版本(或指定 revision)。",
    input_schema: &[
        ParamSpec { name: "revision", kind: ParamKind::Integer, required: false },
    ],
    propagation: &[
        PropagationRule { edge: "CONTAINS", direction: Direction::Forward, max_depth: 1, target_type: Some("Pod"), impact: Impact::Medium, note: "所有 Pod 滚动重启" },
        PropagationRule { edge: "ROUTES_TO", direction: Direction::Reverse, max_depth: 2, target_type: Some("Service"), impact: Impact::Medium, note: "滚动期间 Service 部分 Endpoints 切换" },
        PropagationRule { edge: "BELONGS_TO", direction: Direction::Forward, max_depth: 2, target_type: None, impact: Impact::Medium, note: "Component / Application 部分流量回退" },
    ],
    sla_impact_estimate: "0.5% - 2%",
    warnings: &[
        "滚动重启期间部分实例不可用,持续 1-3 分钟",
        "回滚到旧版可能引入已知 bug",
        "若 ConfigMap 已升级,旧版本可能与新配置不兼容",
    ],
};

pub static REFRESH_SECRET: ActionDef = ActionDef {
    action_id: "refresh_secret",
    name: "刷新 Secret",
    category: "config",
    target_type: "Secret",
    risk_level: RiskLevel::Medium,
    requires_approval: true,
    rollback_action_id: None,
    estimated_duration_seconds: 300,
    description: "更新 Secret 内容并(可选)滚动重启所有引用它的 Pod。",
    input_schema: &[
        ParamSpec { name: "trigger_pod_restart", kind: ParamKind::Boolean, required: false },
    ],
    propagation: &[
        PropagationRule { edge: "USES", direction: Direction::Reverse, max_depth: 2, target_type: Some("Pod"), impact: Impact::Medium, note: "所有引用此 Secret 的 Pod 滚动重启" },
        PropagationRule { edge: "USES", direction: Direction::Reverse, max_depth: 1, target_type: Some("Deployment"), impact: Impact::Low, note: "Deployment 触发滚动更新" },
        PropagationRule { edge: "BELONGS_TO", direction: Direction::Forward, max_depth: 3, target_type: None, impact: Impact::Low, note: "Component / Application 滚动期间 SLA 短暂影响" },
    ],
    sla_impact_estimate: "0.1% - 0.5%",
    warnings: &[
        "旧 Secret 一旦覆盖无法回滚,执行前应备份",
        "若新 Secret 内容错误,所有引用 Pod 会启动失败",
    ],
};

pub static DRAIN_NODE: ActionDef = ActionDef {
    action_id: "drain_node",
    name: "驱逐 Node 上的 Pod",
    category: "drain",
    target_type: "KubernetesNode",
    risk_level: RiskLevel::High,
    requires_approval: true,
    rollback_action_id: None,
    estimated_duration_seconds: 600,
    description: "对 Node 执行 cordon + drain,将其上 Pod 迁移到其他节点。",
    input_schema: &[
        ParamSpec { name: "ignore_daemonsets", kind: ParamKind::Boolean, required: false },
        ParamSpec { name: "delete_local_data", kind: ParamKind::Boolean, required: false },
        ParamSpec { name: "force", kind: ParamKind::Boolean, required: false },
    ],
    propagation: &[
        PropagationRule { edge: "SCHEDULED_ON", direction: Direction::Reverse, max_depth: 1, target_type: Some("Pod"), impact: Impact::High, note: "节点上所有 Pod 被驱逐重新调度" },
        PropagationRule { edge: "CONTAINS", direction: Direction::Reverse, max_depth: 2, target_type: Some("Deployment"), impact: Impact::Medium, note: "受影响 Pod 所属 Deployment 触发重新调度" },
        PropagationRule { edge: "BELONGS_TO", direction: Direction::Forward, max_depth: 3, target_type: None, impact: Impact::Medium, note: "受影响应用短暂部分实例不可用" },
    ],
    sla_impact_estimate: "1% - 5%",
    warnings: &[
        "节点上所有 Pod 不可用 5-10 分钟",
        "若集群资源紧张,Pod 重新调度可能失败",
        "DaemonSet Pod 默认保留(ignore_daemonsets=True)",
    ],
};

pub static KILL_QUERY: ActionDef = ActionDef {
    action_id: "kill_query",
    name: "终止 MySQL 慢查询",
    category: "other",
    target_type: "MySQL",
    risk_level: RiskLevel::Medium,
    // 例外:medium 风险但不需审批(查 reference action_defs)
    requires_approval: false,
    rollback_action_id: None,
    estimated_duration_seconds: 5,
    description: "对 MySQL 执行 KILL QUERY,终止特定连接的当前 SQL。",
    input_schema: &[
        ParamSpec { name: "query_id", kind: ParamKind::String, required: true },
        ParamSpec { name: "min_duration_seconds", kind: ParamKind::Integer, required: false },
    ],
    propagation: &[
        PropagationRule { edge: "USES", direction: Direction::Reverse, max_depth: 2, target_type: Some("Pod"), impact: Impact::Low, note: "依赖此 MySQL 的 Pod 该查询失败,客户端需重试" },
        PropagationRule { edge: "BELONGS_TO", direction: Direction::Forward, max_depth: 3, target_type: None, impact: Impact::Low, note: "上游应用收到查询失败响应" },
    ],
    sla_impact_estimate: "0.01% - 0.1%",
    warnings: &[
        "被杀 SQL 已执行的部分会回滚(如果在事务里)",
        "应用需具备重试能力,否则用户感知",
    ],
};

pub static RESTART_SERVICE: ActionDef = ActionDef {
    action_id: "restart_service",
    name: "重启 Service Endpoints",
    category: "availability",
    target_type: "Service",
    risk_level: RiskLevel::Low,
    requires_approval: false,
    rollback_action_id: None,
    estimated_duration_seconds: 30,
    description: "重新生成 Service Endpoints,触发 kube-proxy 同步 iptables。",
    input_schema: &[
        ParamSpec { name: "drop_idle_seconds", kind: ParamKind::Integer, required: false },
    ],
    propagation: &[
        PropagationRule { edge: "ROUTES_TO", direction: Direction::Forward, max_depth: 1, target_type: Some("Pod"), impact: Impact::Minimal, note: "Endpoints 重新生成,Pod 不动" },
        PropagationRule { edge: "BELONGS_TO", direction: Direction::Forward, max_depth: 3, target_type: None, impact: Impact::Minimal, note: "应用层无感" },
    ],
    sla_impact_estimate: "< 0.05%",
    warnings: &["重启期间(< 5 秒)新建连接可能短暂失败"],
};

pub static CLEAR_CACHE: ActionDef = ActionDef {
    action_id: "clear_cache",
    name: "清空 Redis 缓存",
    category: "other",
    target_type: "Redis",
    risk_level: RiskLevel::Medium,
    requires_approval: true,
    rollback_action_id: None,
    estimated_duration_seconds: 60,
    description: "对 Redis 执行 FLUSHDB / SCAN+DEL,清除指定范围缓存。",
    input_schema: &[
        ParamSpec { name: "scope", kind: ParamKind::String, required: false },
        ParamSpec { name: "db_index", kind: ParamKind::Integer, required: false },
        ParamSpec { name: "key_pattern", kind: ParamKind::String, required: false },
    ],
    propagation: &[
        PropagationRule { edge: "USES", direction: Direction::Reverse, max_depth: 2, target_type: Some("Pod"), impact: Impact::High, note: "依赖此 Redis 的 Pod 缓存击穿,负载暴增" },
        PropagationRule { edge: "USES", direction: Direction::Reverse, max_depth: 1, target_type: Some("MySQL"), impact: Impact::High, note: "上游 DB 在缓存击穿后承担直接负载" },
        PropagationRule { edge: "BELONGS_TO", direction: Direction::Forward, max_depth: 3, target_type: None, impact: Impact::Medium, note: "应用响应延迟显著增加,可能引发雪崩" },
    ],
    sla_impact_estimate: "1% - 10%",
    warnings: &[
        "缓存击穿会让 DB 在短时间内承担数倍负载",
        "scope=all/db 风险极高,生产环境建议只用 pattern",
        "建议预热(主动 warm-up)而非裸清空",
    ],
};

/// 8 个动作模板(引用命名 static)。
pub static ACTION_DEFS: &[&ActionDef] = &[
    &RESTART_POD,
    &SCALE_DEPLOYMENT,
    &ROLLBACK_DEPLOYMENT,
    &REFRESH_SECRET,
    &DRAIN_NODE,
    &KILL_QUERY,
    &RESTART_SERVICE,
    &CLEAR_CACHE,
];

/// 取动作模板;不存在返 None。
pub fn get_action(action_id: &str) -> Option<&'static ActionDef> {
    ACTION_DEFS
        .iter()
        .copied()
        .find(|a| a.action_id == action_id)
}

/// 列全部动作。
pub fn list_actions() -> &'static [&'static ActionDef] {
    ACTION_DEFS
}

/// 按目标类型 / 分类 / 风险级别过滤(None = 不筛)。
pub fn list_actions_filtered(
    target_type: Option<&str>,
    category: Option<&str>,
    risk_level: Option<RiskLevel>,
) -> Vec<&'static ActionDef> {
    ACTION_DEFS
        .iter()
        .copied()
        .filter(|a| target_type.is_none_or(|t| a.target_type == t))
        .filter(|a| category.is_none_or(|c| a.category == c))
        .filter(|a| risk_level.is_none_or(|r| a.risk_level == r))
        .collect()
}

/// 推荐动作(含动作模板 + 理由 + 置信度)。
///
/// 只 Serialize(给 Tauri 返回用),不 Deserialize -- `action: &'static ActionDef`
/// 是 static 构造期引用命名 static,引用无法反序列化。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ActionSuggestion {
    /// 推荐的动作模板。
    pub action: &'static ActionDef,
    /// 推荐理由。
    pub rationale: &'static str,
    /// 置信度 0..=1.0。
    pub confidence: f32,
}

/// rule_id -> 推荐动作列表(对齐 reference RULE_ACTION_SUGGESTIONS)。
pub static RULE_ACTION_SUGGESTIONS: &[(&str, &[ActionSuggestion])] = &[
    ("rule-001", &[
        ActionSuggestion { action: &SCALE_DEPLOYMENT, rationale: "Pod CPU 高且单副本 -> 水平扩容缓解", confidence: 0.85 },
        ActionSuggestion { action: &RESTART_POD, rationale: "重启可能临时缓解,不解决根因", confidence: 0.45 },
    ]),
    ("rule-002", &[
        ActionSuggestion { action: &ROLLBACK_DEPLOYMENT, rationale: "频繁重启常因新版本 bug -> 回滚版本", confidence: 0.75 },
        ActionSuggestion { action: &RESTART_POD, rationale: "短期止血,需配合根因排查", confidence: 0.40 },
    ]),
    ("rule-003", &[
        ActionSuggestion { action: &SCALE_DEPLOYMENT, rationale: "副本不足 -> 直接补到期望值", confidence: 0.90 },
        ActionSuggestion { action: &ROLLBACK_DEPLOYMENT, rationale: "若是新版本启动失败 -> 回滚", confidence: 0.55 },
    ]),
    ("rule-004", &[
        ActionSuggestion { action: &REFRESH_SECRET, rationale: "Secret 过期 -> 直接轮换", confidence: 0.95 },
    ]),
    ("rule-005", &[
        ActionSuggestion { action: &ROLLBACK_DEPLOYMENT, rationale: "高危 CVE -> 回滚到无漏洞版本", confidence: 0.80 },
    ]),
    ("rule-006", &[
        ActionSuggestion { action: &RESTART_SERVICE, rationale: "Service 无 Endpoints -> 重新同步", confidence: 0.70 },
    ]),
    ("rule-007", &[
        ActionSuggestion { action: &REFRESH_SECRET, rationale: "TLS 即将过期 -> 轮换证书", confidence: 0.95 },
    ]),
    ("rule-008", &[
        ActionSuggestion { action: &DRAIN_NODE, rationale: "节点压力 -> 驱逐 Pod 到其他节点", confidence: 0.70 },
    ]),
    ("rule-009", &[
        ActionSuggestion { action: &ROLLBACK_DEPLOYMENT, rationale: "ConfigMap 漂移 -> 回滚配置 + 滚动重启", confidence: 0.65 },
    ]),
    ("rule-010", &[
        ActionSuggestion { action: &ROLLBACK_DEPLOYMENT, rationale: "镜像安全配置错误 -> 回滚到合规版本", confidence: 0.50 },
    ]),
];

/// change_type -> 推荐动作列表(对齐 reference CHANGE_ACTION_SUGGESTIONS)。
/// 3.5 engine-changes 用此桥接(PRD-002 -> PRD-001)。
pub static CHANGE_ACTION_SUGGESTIONS: &[(&str, &[ActionSuggestion])] = &[
    ("configmap_updated", &[
        ActionSuggestion { action: &ROLLBACK_DEPLOYMENT, rationale: "ConfigMap 漂移 -> 回滚 Deployment 滚动恢复旧配置", confidence: 0.65 },
    ]),
    ("secret_rotated", &[
        ActionSuggestion { action: &REFRESH_SECRET, rationale: "Secret 轮换后刷新挂载,推动 Pod 重新加载", confidence: 0.80 },
        ActionSuggestion { action: &ROLLBACK_DEPLOYMENT, rationale: "回滚到挂载旧 Secret 版本的 Deployment revision", confidence: 0.55 },
    ]),
    ("deployment_rolled", &[
        ActionSuggestion { action: &ROLLBACK_DEPLOYMENT, rationale: "新版本异常 -> 回滚到上一 revision", confidence: 0.90 },
    ]),
    ("image_pushed", &[
        ActionSuggestion { action: &ROLLBACK_DEPLOYMENT, rationale: "高危镜像 -> 回滚到合规版本", confidence: 0.75 },
    ]),
];

/// 给定 InspectionRule.rule_id,返回推荐动作列表;未知 rule 返空。
pub fn suggest_for_rule(rule_id: &str) -> &'static [ActionSuggestion] {
    RULE_ACTION_SUGGESTIONS
        .iter()
        .find(|(rid, _)| *rid == rule_id)
        .map(|(_, sugs)| *sugs)
        .unwrap_or(&[])
}

/// 给定 ChangeEvent.change_type,返回推荐动作列表;未知 type 返空。
pub fn suggest_for_change(change_type: &str) -> &'static [ActionSuggestion] {
    CHANGE_ACTION_SUGGESTIONS
        .iter()
        .find(|(ct, _)| *ct == change_type)
        .map(|(_, sugs)| *sugs)
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const EXPECTED_IDS: &[&str] = &[
        "restart_pod",
        "scale_deployment",
        "rollback_deployment",
        "refresh_secret",
        "drain_node",
        "kill_query",
        "restart_service",
        "clear_cache",
    ];

    #[test]
    fn eight_actions_defined() {
        let ids: HashSet<&str> = ACTION_DEFS.iter().map(|a| a.action_id).collect();
        assert_eq!(ids, EXPECTED_IDS.iter().copied().collect::<HashSet<_>>());
    }

    #[test]
    fn required_fields_present() {
        for a in ACTION_DEFS {
            assert!(!a.name.is_empty(), "{} name", a.action_id);
            assert!(!a.category.is_empty());
            assert!(!a.target_type.is_empty());
            assert!(!a.description.is_empty());
            assert!(!a.sla_impact_estimate.is_empty());
            assert!(a.estimated_duration_seconds > 0);
            assert!(!a.warnings.is_empty());
            assert!(!a.input_schema.is_empty());
            assert!(!a.propagation.is_empty(), "{} propagation", a.action_id);
            for rule in a.propagation {
                assert!(!rule.edge.is_empty());
                assert!(rule.direction == Direction::Forward || rule.direction == Direction::Reverse);
                assert!(rule.impact.rank() <= 3);
            }
        }
    }

    #[test]
    fn high_risk_requires_approval() {
        for a in ACTION_DEFS {
            if a.risk_level == RiskLevel::High {
                assert!(a.requires_approval, "{} is high risk but requires_approval=false", a.action_id);
            }
        }
    }

    #[test]
    fn kill_query_is_medium_without_approval() {
        let kq = get_action("kill_query").unwrap();
        assert_eq!(kq.risk_level, RiskLevel::Medium);
        assert!(!kq.requires_approval);
    }

    #[test]
    fn list_actions_count_and_filters() {
        assert_eq!(list_actions().len(), 8);
        let deploys: Vec<&str> = list_actions_filtered(Some("Deployment"), None, None)
            .iter().map(|a| a.action_id).collect();
        assert_eq!(deploys, vec!["scale_deployment", "rollback_deployment"]);
        let high: HashSet<&str> = list_actions_filtered(None, None, Some(RiskLevel::High))
            .iter().map(|a| a.action_id).collect();
        assert_eq!(high, ["rollback_deployment", "drain_node"].into_iter().collect::<HashSet<_>>());
        let pods: HashSet<&str> = list_actions_filtered(Some("Pod"), None, None)
            .iter().map(|a| a.action_id).collect();
        assert_eq!(pods, ["restart_pod"].into_iter().collect());
    }

    #[test]
    fn suggest_for_rule_known_and_unknown() {
        let sugs = suggest_for_rule("rule-001");
        let ids: Vec<&str> = sugs.iter().map(|s| s.action.action_id).collect();
        assert!(ids.contains(&"scale_deployment"));
        assert!(ids.contains(&"restart_pod"));
        for s in sugs {
            assert!(s.confidence > 0.0 && s.confidence <= 1.0);
            assert!(!s.rationale.is_empty());
        }
        assert!(suggest_for_rule("rule-9999").is_empty());
    }

    #[test]
    fn suggest_for_change_known() {
        let sugs = suggest_for_change("deployment_rolled");
        assert_eq!(sugs.len(), 1);
        assert_eq!(sugs[0].action.action_id, "rollback_deployment");
        assert!(suggest_for_change("unknown_type").is_empty());
    }
}
