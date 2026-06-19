#!/usr/bin/env bash
# Deploy OpenTelemetry Demo 0.32.0 (slim) on the vm K8s cluster.
#
# 用法:
#   bash scripts/otel_demo/deploy.sh           # 默认连 vm 集群,装 slim 版
#   bash scripts/otel_demo/deploy.sh --full    # 装完整版(需 8GB+ 内存)
#
# 依赖:
#   - SSH 配置 vm1(已在 ~/.ssh/config)
#   - vm1 上已装 helm v4+
#   - 集群已有 default StorageClass(local-path)
#
# 锁定 chart 版本 0.32.0(对应 OTel Demo 1.13.x app)。

set -euo pipefail

CHART_VERSION="0.32.0"
NAMESPACE="otel-demo"
RELEASE="otel-demo"
SSH_HOST="${SSH_HOST:-vm1}"
VALUES_FILE="${VALUES_FILE:-scripts/otel_demo/values-slim.yaml}"

# --------------------------------------------------------
# Parse args
# --------------------------------------------------------
USE_FULL=0
for arg in "$@"; do
  case $arg in
    --full)
      USE_FULL=1
      shift
      ;;
  esac
done

# --------------------------------------------------------
# Sanity check
# --------------------------------------------------------
echo "==> 检查 vm1 集群状态"
ssh "$SSH_HOST" "kubectl get nodes --no-headers | wc -l" | grep -q '^3$' || {
  echo "ERROR: vm 集群节点不全 3 个" >&2
  exit 1
}

ssh "$SSH_HOST" "helm version --short" >/dev/null || {
  echo "ERROR: vm1 上没装 helm" >&2
  exit 1
}

# --------------------------------------------------------
# Helm repo
# --------------------------------------------------------
echo "==> 添加 OTel Helm repo"
ssh "$SSH_HOST" "
  helm repo add open-telemetry https://open-telemetry.github.io/opentelemetry-helm-charts 2>/dev/null || true
  helm repo update
"

# --------------------------------------------------------
# 上传 values 文件
# --------------------------------------------------------
if [[ $USE_FULL -eq 0 ]]; then
  echo "==> 上传 slim values"
  scp "$VALUES_FILE" "$SSH_HOST:/tmp/otel-demo-values.yaml"
  VALUES_FLAG="-f /tmp/otel-demo-values.yaml"
else
  echo "==> 完整版(无自定义 values)"
  VALUES_FLAG=""
fi

# --------------------------------------------------------
# Install / Upgrade
# --------------------------------------------------------
echo "==> helm upgrade --install (chart $CHART_VERSION)"
ssh "$SSH_HOST" "
  kubectl create namespace $NAMESPACE --dry-run=client -o yaml | kubectl apply -f -
  helm upgrade --install $RELEASE open-telemetry/opentelemetry-demo \
    --version $CHART_VERSION \
    --namespace $NAMESPACE \
    $VALUES_FLAG \
    --timeout 10m \
    --wait=false
"

# --------------------------------------------------------
# 等 Pod
# --------------------------------------------------------
echo "==> 等待 Pod 就绪(最多 5 分钟)"
ssh "$SSH_HOST" "
  for i in \$(seq 1 60); do
    not_ready=\$(kubectl get pod -n $NAMESPACE --no-headers 2>/dev/null | grep -vE '(Running|Completed)' | wc -l)
    total=\$(kubectl get pod -n $NAMESPACE --no-headers 2>/dev/null | wc -l)
    echo \"  [\${i}/60] \${total} pods, \${not_ready} 未就绪\"
    if [[ \$total -gt 10 && \$not_ready -eq 0 ]]; then
      break
    fi
    sleep 5
  done
"

# --------------------------------------------------------
# 摘要
# --------------------------------------------------------
echo "==> 部署摘要"
ssh "$SSH_HOST" "
  kubectl get pod -n $NAMESPACE -o wide
  echo
  kubectl get svc -n $NAMESPACE
"

echo
echo "==> ✅ 完成"
echo "下一步访问:"
echo "  ssh $SSH_HOST 'kubectl port-forward -n $NAMESPACE svc/frontend-proxy 8080:8080'"
echo "  → 浏览器打开 http://localhost:8080"
