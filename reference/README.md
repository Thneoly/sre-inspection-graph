# reference/ — Python 行为参考实现(read-only)

> ⚠️ **此目录为只读** — 自 Phase 1 起,本目录不接受功能改动、bug 修复、依赖升级。
> 仅作为 Rust engine/ 实现的**行为参考**(behavior oracle),Phase 5 完工后 `git rm`。

## 为什么保留

- **472 个测试** + **71 个前端测试** 是 PRD-001/002/003/004 的执行级规约 — 即"应该怎么跑"的最权威记录
- 与其凭 PRD 文档反推语义,不如开两个终端 diff:Python ↔ Rust 同输入应同输出
- 复刻的每一个模块,都用 `tests/contract/parity_<module>.rs` 把 reference 输出当 golden 对照

## 怎么用

```bash
# 在 reference/ 跑 Python 测试(保留 dev 环境)
cd reference && uv sync
cd reference && uv run python -m pytest -p no:asyncio -q

# 跑特定模块(用于 parity test)
cd reference && uv run python -m pytest tests/test_recovery.py -p no:asyncio -v

# 启动 reference API(作为 Rust engine 的 oracle,本地 dev 用)
cd reference && uv run uvicorn app.main:app --port 8001
# 注:端口 8001 区别于将来 engine-cli 的 8000
```

## 纪律

| 禁止 | 例外 |
|---|---|
| 修复 reference/ 里的 bug | 如果 bug 影响 contract test 的 golden,可改 — 但必须同步加注释 `# REF-FIX: <date> <why>` |
| 加新功能到 reference/ | 没有例外。新功能直接进 engine/ |
| 升级 Python 依赖 | 没有例外。pyproject.toml 冻结 |
| 改 frontend/(将迁 desktop/src) | frontend/ Phase 1 起视作 read-only;所有改动在 desktop/src 进行 |

## 复刻进度

见 [`MIGRATION_STATUS.md`](./MIGRATION_STATUS.md)。

## 退役条件

- 全部 4 个 PRD 复刻完成 + Rust contract test 全绿 + reference/ 至少 30 天没被读
- 满足 → Phase 5 `git rm -rf reference/`,并在 doc/14 时间线末尾记一笔
- 不满足 → 保留到 v1.1 release,但不延期 Phase 5 收口

---

**最后更新**:2026-06-23(Phase 1 启动,backend/ 重命名为 reference/)
