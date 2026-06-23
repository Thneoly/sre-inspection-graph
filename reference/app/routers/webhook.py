"""Webhook 接收 — PRD-002 Phase 2。

接收外部系统推送的变更事件,转成 ChangeEvent 入库。PoC 简化:
- 可选 `WEBHOOK_TOKEN` header 校验(空则跳过 —— 生产必开)
- 不做 HMAC 签名校验(留 Phase 3)

端点(prefix `/api/v1/webhooks`):
- POST /argocd  — Argo CD application-sync webhook → deployment_rolled
- POST /harbor  — Harbor 镜像推送 webhook → image_pushed
"""
from __future__ import annotations

import logging
from typing import Any, Optional

from fastapi import APIRouter, Header, HTTPException, Request

from app.changes.event_service import ChangeEventError, record_change, serialize
from app.config import settings
from app.datasource.store import store


logger = logging.getLogger(__name__)

router = APIRouter(prefix="/api/v1/webhooks", tags=["Webhooks"])


def _check_token(x_webhook_token: Optional[str]) -> None:
    """可选 token 校验。settings.webhook_token 空 → 跳过(PoC)。"""
    if not settings.webhook_token:
        return
    if x_webhook_token != settings.webhook_token:
        raise HTTPException(status_code=401, detail="invalid webhook token")


def _find_deployment_by_name(name: str, cluster_id: str = "") -> Optional[str]:
    """在 DSS 里按 Deployment name 找 node_id。找不到返 None(占位 ID 仍记录)。"""
    if not name:
        return None
    for node in store.nodes.values():
        if node.type == "Deployment" and node.name == name:
            return node.id
    # 退而用 k8s_mapper 构造占位 ID(target 不在 DSS 也记录)
    from app.datasource.connectors.k8s_mapper import deployment_id
    ns = settings.k8s_namespace
    return deployment_id(cluster_id or settings.active_cluster, ns, name)


@router.post("/argocd", status_code=201)
async def argocd_webhook(
    request: Request,
    x_webhook_token: Optional[str] = Header(None, alias="X-Webhook-Token"),
):
    """Argo CD application sync webhook。

    兼容 Argo CD notification webhook payload(取 application 名 + sync revision)。
    payload 结构宽松解析,缺关键字段返 400。
    """
    _check_token(x_webhook_token)
    try:
        payload = await request.json()
    except Exception as e:  # noqa: BLE001
        raise HTTPException(status_code=400, detail=f"invalid JSON: {e}")

    # Argo CD payload 常见结构:{application: {metadata: {name}, spec: {source: {repoURL}}},
    #                          revision, sync_status: {revision}}
    app = payload.get("application") or payload.get("app") or {}
    meta = app.get("metadata", {}) if isinstance(app, dict) else {}
    app_name = meta.get("name") or payload.get("app_name") or ""
    spec = app.get("spec", {}) if isinstance(app, dict) else {}
    source = spec.get("source", {}) if isinstance(spec, dict) else {}
    repo_url = source.get("repoURL") or payload.get("repo_url") or ""

    # revision 优先 sync_status.revision,其次 payload.revision
    revision = (
        (payload.get("sync_status") or {}).get("revision")
        or payload.get("revision")
        or ""
    )

    # 镜像变更(若 payload 带 Images 字段)
    images = payload.get("images") or []

    if not app_name:
        raise HTTPException(status_code=400, detail="missing application name")

    target_id = _find_deployment_by_name(app_name) or app_name

    diff_summary: dict[str, Any] = {"application": app_name}
    if images:
        diff_summary["images"] = images
    if revision:
        diff_summary["revision"] = revision[:12]

    try:
        event = record_change(
            change_type="deployment_rolled",
            target_resource_id=target_id,
            changed_by="argo-cd",
            source="argo_cd",
            description=f"Argo CD sync: {app_name} @ {revision[:12] if revision else 'unknown'}",
            diff_summary=diff_summary,
            commit_sha=revision,
            git_repo=repo_url,
            cluster_id=settings.active_cluster,
        )
    except ChangeEventError as e:
        raise HTTPException(status_code=e.code, detail=str(e))
    return serialize(event)


@router.post("/harbor", status_code=201)
async def harbor_webhook(
    request: Request,
    x_webhook_token: Optional[str] = Header(None, alias="X-Webhook-Token"),
):
    """Harbor 镜像推送 webhook → image_pushed。

    Harbor webhook payload 结构:{type: "PUSH_ARTIFACT",
      event_data: {repository: {repo_full_name, namespace}, resources: [{resource: {digest, tag}}]}}
    """
    _check_token(x_webhook_token)
    try:
        payload = await request.json()
    except Exception as e:  # noqa: BLE001
        raise HTTPException(status_code=400, detail=f"invalid JSON: {e}")

    event_data = payload.get("event_data", {}) or {}
    repo = event_data.get("repository", {}) or {}
    repo_full = repo.get("repo_full_name") or repo.get("name") or ""
    resources = event_data.get("resources", []) or []
    if not resources:
        raise HTTPException(status_code=400, detail="missing resources in payload")

    results = []
    for res in resources:
        res_info = res.get("resource", {}) if isinstance(res, dict) else {}
        tag = res_info.get("tag", "")
        digest = res_info.get("digest", "")
        image_ref = f"{repo_full}:{tag}" if tag else repo_full

        # target 指向 ContainerImage 节点(若 DSS 无则占位 ID)
        target_id = _find_image_node(repo_full, tag) or f"img:{image_ref}"

        try:
            event = record_change(
                change_type="image_pushed",
                target_resource_id=target_id,
                changed_by="harbor-webhook",
                source="gitops",
                description=f"Image pushed: {image_ref}",
                diff_summary={
                    "repository": repo_full,
                    "tag": tag,
                    "digest": digest[:19] if digest else "",
                },
                cluster_id=settings.active_cluster,
            )
            results.append(serialize(event))
        except ChangeEventError as e:
            raise HTTPException(status_code=e.code, detail=str(e))

    return {"events": results, "total": len(results)}


def _find_image_node(repo_full: str, tag: str) -> Optional[str]:
    """在 DSS 找 ContainerImage 节点。找不到返 None。"""
    if not repo_full:
        return None
    ref = f"{repo_full}:{tag}" if tag else repo_full
    for node in store.nodes.values():
        if node.type == "ContainerImage":
            # 简单匹配:name 或 properties.image 含 repo_full
            if repo_full in (node.name or "") or repo_full in str(node.properties.get("image", "")):
                return node.id
    return None
