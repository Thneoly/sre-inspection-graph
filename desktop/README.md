# desktop/

Tauri 2.x 桌面应用 — Phase 1 起取代 SaaS Web 形态。

## 目录

```
desktop/
├── package.json            # React + Vite + AntD + Cytoscape
├── vite.config.ts
├── tsconfig.json
├── index.html
├── src/                    # React + TypeScript 前端(Phase 2 起从 frontend/ 迁 ~90%)
│   ├── main.tsx
│   ├── App.tsx
│   ├── api/
│   │   ├── client.ts       # axios → tauri invoke 适配层
│   │   └── generated.ts    # tauri-specta 自动生成的 TS 类型(Phase 2)
│   └── styles.css
└── src-tauri/              # Rust 部分(Tauri backend)
    ├── Cargo.toml          # path deps:engine-core / engine-storage / engine-wasm
    ├── tauri.conf.json     # 应用元数据 / 权限 / CSP
    ├── build.rs
    └── src/
        ├── main.rs
        ├── lib.rs
        ├── state.rs        # AppState
        └── commands/       # tauri commands 按领域分文件
            ├── mod.rs
            └── system.rs   # 第一个示例 command:get_app_version
```

## 启动

```bash
# 一次性
cd desktop && npm install

# 开发模式(热重载 — Vite + Tauri 同时跑)
cd desktop && npm run tauri dev
# 或 make desktop-dev

# 打包(release)
cd desktop && npm run tauri build
# 或 make desktop-build
# 产物在 desktop/src-tauri/target/release/bundle/<platform>/
```

## Phase 1 状态

- 骨架完成,1 个示例 command `get_app_version`(返回 engine-core 版本)
- 前端目录仅含 `App.tsx` 骨架展示 command 返回值
- 详细架构(commands / IPC / 存储 schema / 安全模型 / 打包)见
  [`doc/17-tauri-desktop-architecture.md`](../doc/17-tauri-desktop-architecture.md)

## Phase 2 工作

- 从 `frontend/src/` 迁移到 `desktop/src/`(API 层改 `invoke`)
- 接入 tauri-specta TS 类型生成
- 接入 engine-storage SQLite 本地持久化
