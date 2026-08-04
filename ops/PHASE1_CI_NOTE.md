# ops/ — Phase 1 CI 草稿暂存

`ci.yml.draft` 和 `README.md.draft` 是 GitHub Actions CI 工作流草稿,
**暂时不放在 `.github/workflows/` 下**:推送此分支用的 OAuth credential 缺
`workflow` scope,GitHub 服务端拒绝。

## 启用方式

把两个 draft 文件改回 `.github/workflows/` 即可生效:

```bash
mkdir -p .github/workflows
git mv ops/ci.yml.draft       .github/workflows/ci.yml
git mv ops/README.md.draft    .github/workflows/README.md
git rm  ops/PHASE1_CI_NOTE.md
git commit -m "ci: enable Phase 1 GitHub Actions workflow"
git push  # 需要 PAT/SSH 持 workflow scope
```

合并 PR 之前由 repo owner 用本地 SSH 或带 `workflow` scope 的 PAT 推上去,
**不要**让 OAuth bot 推。

## 文件内容

| 文件 | 内容 |
|---|---|
| `ci.yml.draft` | 4 job 并行:engine / modules / desktop / lockfile |
| `README.md.draft` | CI 设计说明 |
