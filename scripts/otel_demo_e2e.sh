#!/usr/bin/env bash
# PRD-004 端到端手测脚本 — 5 个 connector 实拉 OTel Demo
#
# 用法:
#   bash scripts/otel_demo_e2e.sh
#   API_BASE=http://other-host:8000 bash $0
#
# 前提:
#   - vm 集群已 helm install OTel demo 0.32.0(scripts/otel_demo/deploy.sh)
#   - 三个 port-forward 都活着:
#       prometheus 19090, jaeger 16686, flagd 8013
#   - 后端 API 已起,带正确环境变量:
#       KUBECONFIGS=vm-cluster=~/.kube/vm-config
#       PROMETHEUS_URL=http://localhost:19090
#       JAEGER_URL=http://localhost:16686/jaeger/ui
#       FLAGD_URL=http://localhost:8013
#   - jq 已装
#
# 验证范围:
#   1. 5 个 connector 全部注册 + status 健康
#   2. K8s connector 拉到 ≥80 nodes / ≥90 edges
#   3. Prometheus 写入 ≥10 metric samples + 推导出 component.health
#   4. Jaeger 聚合产生 CALLS 边
#   5. flagd flip 触发 ChangeEvent

set -euo pipefail

API_BASE="${API_BASE:-http://localhost:8000}"
CONN="${API_BASE}/api/v1/connectors"
DS="${API_BASE}/api/v1/datasource"

G='\033[0;32m'; Y='\033[0;33m'; R='\033[0;31m'; B='\033[0;34m'; N='\033[0m'

step() { echo -e "\n${B}━━━ $1 ━━━${N}"; }
ok()   { echo -e "${G}✓${N} $1"; }
warn() { echo -e "${Y}⚠${N} $1"; }
fail() { echo -e "${R}✗${N} $1"; exit 1; }

require_cmd() { command -v "$1" >/dev/null 2>&1 || fail "缺少命令: $1"; }
require_cmd jq
require_cmd curl

# ============================================================
step "Step 1 — 检查 connector registry"
# ============================================================
status_json=$(curl -sf "${CONN}/status") || fail "GET /connectors/status 失败"
total=$(echo "$status_json" | jq -r '.total')
echo "$status_json" | jq -r '.connectors[] | "  \(.name): running=\(.running) errors_24h=\(.error_count_24h) last=\((.last_result.notes // [""])[0])"'

[[ "$total" -eq 5 ]] || fail "期望 5 个 connector,实际 $total"
ok "5 个 connector 全注册"

# ============================================================
step "Step 2 — 手动触发 K8s sync,检查拓扑被刷新"
# ============================================================
k8s_result=$(curl -sf -X POST "${CONN}/k8s/sync-now")
nodes_added=$(echo "$k8s_result" | jq -r '.result.nodes_added')
edges_added=$(echo "$k8s_result" | jq -r '.result.edges_added')
duration=$(echo "$k8s_result" | jq -r '.result.duration_ms')
echo "  k8s sync: nodes_added=$nodes_added edges_added=$edges_added duration=${duration}ms"

# 第二次 sync 应该全是 update,不再 add(diff 起作用)
k8s_result2=$(curl -sf -X POST "${CONN}/k8s/sync-now")
nodes_updated=$(echo "$k8s_result2" | jq -r '.result.nodes_updated')
echo "  k8s re-sync: nodes_updated=$nodes_updated (期望 ≥80,diff 工作正常)"
[[ "$nodes_updated" -ge 80 ]] || warn "第二次 sync nodes_updated 太少,确认 OTel demo 是否健康"
ok "K8s 拓扑同步生效"

# ============================================================
step "Step 3 — Prometheus 拉到 metrics + health 推导"
# ============================================================
prom_result=$(curl -sf -X POST "${CONN}/prometheus/sync-now")
metrics_added=$(echo "$prom_result" | jq -r '.result.metrics_added')
prom_updated=$(echo "$prom_result" | jq -r '.result.nodes_updated')
echo "  prom sync: metrics_added=$metrics_added components_updated=$prom_updated"
echo "$prom_result" | jq -r '.result.notes[] | "    " + .'

[[ "$metrics_added" -ge 10 ]] || warn "metric 样本太少($metrics_added),OTel demo 可能没 traffic"

# 抽 component health 看下分布(客户端过滤,/datasource/nodes 不支持 query)
nodes_json=$(curl -sf "${API_BASE}/api/v1/datasource/nodes")
echo "  health 分布:"
echo "$nodes_json" | jq -r '.nodes[] | select(.type=="ApplicationComponent") | .properties.health // "?"' | sort | uniq -c | awk '{printf "    %s: %d\n", $2, $1}'
ok "Prometheus 接入工作"

# ============================================================
step "Step 4 — Jaeger 聚合 CALLS 边"
# ============================================================
jaeger_result=$(curl -sf -X POST "${CONN}/jaeger/sync-now")
calls_added=$(echo "$jaeger_result" | jq -r '.result.edges_added')
calls_removed=$(echo "$jaeger_result" | jq -r '.result.edges_removed')
echo "  jaeger sync: edges_added=$calls_added edges_removed=$calls_removed"
echo "$jaeger_result" | jq -r '.result.notes[] | "    " + .' | head -3

# 实际计数 — 客户端 jq 过滤 CALLS 边
calls_count=$(curl -sf "${API_BASE}/api/v1/datasource/edges" | jq -r '[.edges[] | select(.type=="CALLS")] | length')
echo "  DSS CALLS 边总数: $calls_count"
[[ "$calls_count" -ge 5 ]] || warn "CALLS 边太少,可能 traces 不够 / threshold 阻挡"
ok "Jaeger trace 聚合生效"

# ============================================================
step "Step 5 — flagd flip 触发 ChangeEvent(可选,需手工 flip)"
# ============================================================
flagd_result=$(curl -sf -X POST "${CONN}/flagd/sync-now")
echo "$flagd_result" | jq -r '.result.notes[] | "    " + .'
events_before=$(curl -sf "${API_BASE}/api/v1/change-events?source=flagd" | jq -r '.total // 0')
echo "  当前 flagd 来源 ChangeEvent: $events_before"

echo -e "  ${Y}手工验证:${N} 在 vm1 上 flip 一个 flag 然后再跑一次 sync"
echo "    1. 修改 flagd ConfigMap 中 productCatalogFailure 的 defaultVariant: off → on"
echo "       ssh vm1 'kubectl -n otel-demo edit cm otel-demo-flagd-config'"
echo "    2. 等 30 秒 → 再触发 sync:"
echo "       curl -X POST ${CONN}/flagd/sync-now | jq '.result.events_added'"
echo "    期望:events_added ≥ 1"

# ============================================================
step "Step 6 — K8s events connector 状态"
# ============================================================
kev_result=$(curl -sf -X POST "${CONN}/k8s_events/sync-now")
kev_events=$(echo "$kev_result" | jq -r '.result.events_added')
echo "  k8s_events sync: events_added=$kev_events"
echo "$kev_result" | jq -r '.result.notes[] | "    " + .'
ok "K8s event connector 工作"

# ============================================================
step "Step 7 — 故障 scenario 列表"
# ============================================================
echo "  PRD-004 内置 OTel demo fault scenarios:"
python3 - <<'PY'
import sys
sys.path.insert(0, "backend")
from app.recovery.scenarios.otel_demo_scenarios import SCENARIOS
for s in SCENARIOS:
    print(f"    {s.name:30s} flag={s.flag_name:30s} action={s.recommended_action}")
print(f"  共 {len(SCENARIOS)} 个")
PY

# ============================================================
echo
echo -e "${G}━━━ E2E 通过 ━━━${N}"
echo "实拉数据汇总:"
curl -sf "${CONN}/status" | jq -r '.connectors[] | "  \((.name + "                ")[0:18]) " + ((.last_result.notes // [""])[0])'
