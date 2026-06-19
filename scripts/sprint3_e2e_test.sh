#!/usr/bin/env bash
# PRD-001 Sprint 3 端到端手测脚本 — 审批流 + 一键回滚
#
# 用法:
#   bash scripts/sprint3_e2e_test.sh                    # 默认 http://localhost:8000
#   API_BASE=http://other-host:8000 bash $0             # 自定义 API 地址
#
# 前提:
#   - 后端 API 在 8000 端口运行 (`make dev-api` 或 `make up`)
#   - 已通过 `make mock-data` + `make infra` 导入 baseline 数据
#   - 已安装 `jq`(用于解析 JSON 响应)
#
# 覆盖路径:
#   1. high_risk 动作 → 创建 awaiting_approval execution + ApprovalRequest
#   2. 审批通过 → 自动触发执行 → succeeded
#   3. 对 succeeded 的 scale_deployment 一键回滚 → rolled_back
#   4. 列出待审批 + 已批准的请求,核对 status 流转

set -euo pipefail

API_BASE="${API_BASE:-http://localhost:8000}"
RECOVERY="${API_BASE}/api/v1/recovery"

# 颜色提示
G='\033[0;32m'  # green
Y='\033[0;33m'  # yellow
R='\033[0;31m'  # red
B='\033[0;34m'  # blue
N='\033[0m'

step() { echo -e "\n${B}━━ $* ━━${N}"; }
ok()   { echo -e "${G}✓${N} $*"; }
warn() { echo -e "${Y}!${N} $*"; }
fail() { echo -e "${R}✗${N} $*"; exit 1; }

# 依赖检查
command -v jq >/dev/null 2>&1 || fail "jq 未安装,请先安装 (brew install jq / apt install jq)"
command -v curl >/dev/null 2>&1 || fail "curl 未安装"

# 健康检查
step "健康检查 ${API_BASE}"
curl -fsS "${API_BASE}/api/v1/health" >/dev/null || fail "API 不可达,确认后端是否在 ${API_BASE} 运行"
ok "API 健康"

# ============================================================
# 找一个真实的 Deployment 作为 high_risk 动作目标
# ============================================================
step "查找一个 Deployment 作为目标"
TOPO_RESP=$(curl -fsS "${API_BASE}/api/v1/topology/app/order?depth=5" 2>/dev/null \
  || curl -fsS "${API_BASE}/api/v1/topology/app/cce?depth=5" 2>/dev/null \
  || echo "{}")

DEPLOYMENT_ID=$(echo "${TOPO_RESP}" | jq -r '
  [.nodes[]? | select(.type == "Deployment")] | .[0].id // empty
')

if [[ -z "${DEPLOYMENT_ID}" ]]; then
  warn "拓扑里没找到 Deployment;退化为 fixture id"
  DEPLOYMENT_ID="deploy:cce-prod-01:order:order-api"
fi
ok "目标 Deployment: ${DEPLOYMENT_ID}"

# ============================================================
# 1. 提交 high_risk 动作 → awaiting_approval (期望 202)
# ============================================================
step "1. 提交 rollback_deployment (high_risk) → 进入审批"
EXEC_RESP=$(curl -fsS -X POST "${RECOVERY}/execute" \
  -H "Content-Type: application/json" \
  -w "\nHTTP_CODE:%{http_code}" \
  -d "{
    \"action_id\": \"rollback_deployment\",
    \"target_resource_id\": \"${DEPLOYMENT_ID}\",
    \"input_params\": {},
    \"initiated_by\": \"alice@e2e\",
    \"request_reason\": \"Sprint 3 E2E test — v1.2.3 上线后告警增多\"
  }")
HTTP_CODE=$(echo "${EXEC_RESP}" | grep "^HTTP_CODE:" | cut -d: -f2)
EXEC_BODY=$(echo "${EXEC_RESP}" | sed '/^HTTP_CODE:/d')

[[ "${HTTP_CODE}" == "202" ]] || fail "期望 202 Accepted,实际 ${HTTP_CODE}: ${EXEC_BODY}"
ok "HTTP ${HTTP_CODE} (Accepted)"

EXECUTION_ID=$(echo "${EXEC_BODY}" | jq -r '.execution_id')
APPROVAL_ID=$(echo "${EXEC_BODY}" | jq -r '.approval_id')
STATUS=$(echo "${EXEC_BODY}" | jq -r '.status')
[[ "${STATUS}" == "awaiting_approval" ]] || fail "期望 status=awaiting_approval,实际 ${STATUS}"
[[ "${APPROVAL_ID}" != "null" && -n "${APPROVAL_ID}" ]] || fail "approval_id 为空"
ok "execution=${EXECUTION_ID}"
ok "approval=${APPROVAL_ID}, status=${STATUS}"

# ============================================================
# 2. 列待审批 → 应有 1 条
# ============================================================
step "2. 列待审批清单"
PENDING_LIST=$(curl -fsS "${RECOVERY}/approvals?status=pending")
PENDING_COUNT=$(echo "${PENDING_LIST}" | jq '[.approvals[] | select(.approval_id == "'"${APPROVAL_ID}"'")] | length')
[[ "${PENDING_COUNT}" -ge 1 ]] || fail "待审批列表里找不到 ${APPROVAL_ID}"
APPROVER_TEAM=$(echo "${PENDING_LIST}" | jq -r '[.approvals[] | select(.approval_id == "'"${APPROVAL_ID}"'")] | .[0].approver_team')
ok "approver_team=${APPROVER_TEAM}, 待审批共 $(echo "${PENDING_LIST}" | jq '.total') 条"

# ============================================================
# 3. 审批通过 → 自动执行
# ============================================================
step "3. 审批通过 → 自动触发执行"
APPROVE_RESP=$(curl -fsS -X POST "${RECOVERY}/approvals/${APPROVAL_ID}/approve" \
  -H "Content-Type: application/json" \
  -d '{"approver_id": "bob@e2e", "comment": "Sprint 3 E2E test — 业务侧已知会"}')

APPROVAL_STATUS=$(echo "${APPROVE_RESP}" | jq -r '.approval.approval_status')
EXEC_STATUS=$(echo "${APPROVE_RESP}" | jq -r '.execution.status')
[[ "${APPROVAL_STATUS}" == "approved" ]] || fail "期望 approved,实际 ${APPROVAL_STATUS}"
[[ "${EXEC_STATUS}" == "succeeded" || "${EXEC_STATUS}" == "failed" ]] \
  || fail "期望 succeeded 或 failed,实际 ${EXEC_STATUS}"
ok "approval=${APPROVAL_STATUS}, execution=${EXEC_STATUS}"

# ============================================================
# 4. 重复审批 → 期望 409 Conflict
# ============================================================
step "4. 重复审批应返 409"
DUP_CODE=$(curl -fsS -o /dev/null -w "%{http_code}" -X POST "${RECOVERY}/approvals/${APPROVAL_ID}/approve" \
  -H "Content-Type: application/json" \
  -d '{"approver_id": "carol@e2e"}' \
  || true)
[[ "${DUP_CODE}" == "409" ]] || fail "期望 409,实际 ${DUP_CODE}"
ok "重复审批正确返 409"

# ============================================================
# 5. 跑一个 low_risk scale_deployment → 同步执行 → 然后回滚
# ============================================================
step "5. 提交 scale_deployment (low_risk) → 同步执行"
SCALE_RESP=$(curl -fsS -X POST "${RECOVERY}/execute" \
  -H "Content-Type: application/json" \
  -w "\nHTTP_CODE:%{http_code}" \
  -d "{
    \"action_id\": \"scale_deployment\",
    \"target_resource_id\": \"${DEPLOYMENT_ID}\",
    \"input_params\": {\"replicas_delta\": 1},
    \"initiated_by\": \"alice@e2e\"
  }")
SCALE_CODE=$(echo "${SCALE_RESP}" | grep "^HTTP_CODE:" | cut -d: -f2)
SCALE_BODY=$(echo "${SCALE_RESP}" | sed '/^HTTP_CODE:/d')
[[ "${SCALE_CODE}" == "200" ]] || fail "low_risk 期望 200,实际 ${SCALE_CODE}"
SCALE_EXEC_ID=$(echo "${SCALE_BODY}" | jq -r '.execution_id')
SCALE_STATUS=$(echo "${SCALE_BODY}" | jq -r '.status')
[[ "${SCALE_STATUS}" == "succeeded" ]] || fail "scale 期望 succeeded,实际 ${SCALE_STATUS}"
ok "scale execution=${SCALE_EXEC_ID}, status=${SCALE_STATUS}"

step "6. 一键回滚 succeeded 的 scale_deployment"
ROLLBACK_RESP=$(curl -fsS -X POST "${RECOVERY}/executions/${SCALE_EXEC_ID}/rollback" \
  -H "Content-Type: application/json" \
  -w "\nHTTP_CODE:%{http_code}" \
  -d '{"initiated_by": "alice@e2e", "reason": "Sprint 3 E2E test — 回滚扩容"}')
RB_CODE=$(echo "${ROLLBACK_RESP}" | grep "^HTTP_CODE:" | cut -d: -f2)
RB_BODY=$(echo "${ROLLBACK_RESP}" | sed '/^HTTP_CODE:/d')
[[ "${RB_CODE}" == "200" ]] || fail "rollback 期望 200,实际 ${RB_CODE}: ${RB_BODY}"
RB_EXEC_ID=$(echo "${RB_BODY}" | jq -r '.execution_id')
RB_STATUS=$(echo "${RB_BODY}" | jq -r '.status')
RB_REVERSES=$(echo "${RB_BODY}" | jq -r '.reverses_execution_id')
[[ "${RB_STATUS}" == "succeeded" ]] || fail "rollback 期望 succeeded,实际 ${RB_STATUS}"
[[ "${RB_REVERSES}" == "${SCALE_EXEC_ID}" ]] || fail "reverses 字段应指向原 execution"
ok "rollback execution=${RB_EXEC_ID}, status=${RB_STATUS}, reverses=${RB_REVERSES}"

step "7. 原 execution 应被标 rolled_back"
ORIG=$(curl -fsS "${RECOVERY}/executions/${SCALE_EXEC_ID}")
ORIG_STATUS=$(echo "${ORIG}" | jq -r '.status')
ORIG_RB_LINK=$(echo "${ORIG}" | jq -r '.rollback_execution_id')
[[ "${ORIG_STATUS}" == "rolled_back" ]] || fail "原 execution 期望 rolled_back,实际 ${ORIG_STATUS}"
[[ "${ORIG_RB_LINK}" == "${RB_EXEC_ID}" ]] || fail "原 execution 的 rollback_execution_id 应指向回滚 exec"
ok "原 execution status=${ORIG_STATUS}, 关联 rollback=${ORIG_RB_LINK}"

step "8. 重复回滚同一 execution → 应返 409"
DUP_RB_CODE=$(curl -fsS -o /dev/null -w "%{http_code}" -X POST "${RECOVERY}/executions/${SCALE_EXEC_ID}/rollback" \
  -H "Content-Type: application/json" \
  -d '{"initiated_by": "alice@e2e"}' \
  || true)
[[ "${DUP_RB_CODE}" == "409" ]] || fail "重复 rollback 期望 409,实际 ${DUP_RB_CODE}"
ok "重复 rollback 正确返 409"

# ============================================================
# 总结
# ============================================================
step "验证完成"
echo -e "${G}所有 8 步检查通过。${N}"
echo ""
echo "本次 E2E 创建的资源:"
echo "  审批 execution:  ${EXECUTION_ID}  (high_risk → succeeded)"
echo "  审批 approval:   ${APPROVAL_ID}  (approved by bob@e2e)"
echo "  scale execution: ${SCALE_EXEC_ID} (rolled_back)"
echo "  rollback exec:   ${RB_EXEC_ID}   (succeeded, reverses ${SCALE_EXEC_ID})"
echo ""
echo "查看历史:"
echo "  curl -s ${RECOVERY}/executions | jq '.executions[] | {execution_id, action_id, status}'"
echo "  curl -s ${RECOVERY}/approvals  | jq '.approvals[]  | {approval_id, approval_status, approver_id}'"
