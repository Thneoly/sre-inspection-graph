#!/usr/bin/env bash
# 卸载 OTel Demo
set -euo pipefail
SSH_HOST="${SSH_HOST:-vm1}"
ssh "$SSH_HOST" "
  helm uninstall otel-demo -n otel-demo 2>/dev/null || true
  kubectl delete namespace otel-demo --ignore-not-found
"
echo "==> ✅ OTel Demo 已卸载"
