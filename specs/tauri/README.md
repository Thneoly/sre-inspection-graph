# specs/tauri/

Tauri commands TS 类型生成的输入位于 `desktop/src-tauri/src/commands/*.rs`,
通过 `tauri-specta` 自动生成 `desktop/src/api/generated.ts`。

本目录 Phase 1 暂空 — 等 desktop/ 骨架就绪后(Step 3)在 build.rs 里挂上
specta 生成,本目录仅作版本登记参考。

详见 [`doc/15-data-contract-spec.md`](../../doc/15-data-contract-spec.md) §2 与
[`doc/17-tauri-desktop-architecture.md`](../../doc/17-tauri-desktop-architecture.md)
中 tauri-specta 配置示例。
