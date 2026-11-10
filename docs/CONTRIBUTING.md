# 🤝 Alchemize 贡献指南

感谢关注 alchemize。本文档说明开发环境、TDD 工作流、代码规范与提交/PR 流程。

## 📋 目录

- [欢迎](#-欢迎)
- [环境准备](#-环境准备)
- [开发工作流（TDD）](#-开发工作流tdd)
- [代码规范](#-代码规范)
- [提交与 PR 流程](#-提交与-pr-流程)
- [行为准则](#-行为准则)

## 👋 欢迎

任何规模的贡献都受欢迎：报告缺陷、改进文档、补测试、加引擎。本项目采用 specmark 规格驱动流程（proposal → apply → converge → archive），能力域规格真源在 `specmark/specs/`——改行为前先看对应能力域的 spec 是否需要同步。

## 🧰 环境准备

### 前置条件

- **Rust 1.97.1+**（`Cargo.toml` 的 `rust-version`，edition 2024；下限由 sdforge MSRV 决定）
- 组件：`rustfmt`（代码格式化）、`clippy`（静态分析）、`cargo-llvm-cov`（覆盖率，可选）

```bash
rustup component add rustfmt clippy
cargo install cargo-llvm-cov   # 可选：覆盖率度量
```

### Fork 与 Clone

```bash
# Fork（GitHub 界面）后：
git clone https://github.com/<you>/alchemize.git
cd alchemize
git remote add upstream https://github.com/Kirky-X/alchemize.git
```

### 完成环境搭建

```bash
# 安装项目依赖并构建
cargo build

# full 档依赖同工作区兄弟仓 vecboost（path 依赖 ../vecboost）
cargo build --features full
```

注意：`full` 档要求 `../vecboost` 存在（path 依赖）；发布顺序 vecboost 先于 alchemize 发到 crates.io。

### 特性标志

| feature | 引入 | 说明 |
| --- | --- | --- |
| `pdf-cloud` | MinerU 云端 PDF 引擎 | 需 `ALCHEMIZE_MINERU_URL` 配置端点才注册 |
| `audio` | vecboost 转录门面（Qwen3-ASR ONNX + silero VAD） | 需模型 bundle + `ORT_DYLIB_PATH` |
| `mcp` | MCP 工具面（alch-mcp，rmcp stdio） | CLI 封装（sdforge cli）恒在 |
| `html` | html-deep 自研引擎（`document/html_deep/engine/`） | scraper/ego-tree/dom_smoothie/icu_properties 同批可选 |
| `full` | 以上全部 | |

### 构建与测试

```bash
cargo build                 # 构建项目
cargo test                  # 运行全部测试（default 档，448 例）
cargo test --features full  # full 档（774 例）
```

质量门禁（本地 pre-commit hook + GitHub CI 双层执行，规则见 `.pre-commit-config.yaml` 与 `.github/workflows/ci.yml`）：

```bash
pre-commit run --all-files  # 本地全量：fmt/clippy(-D warnings)/deny/typos/私钥检测
```

## 🔄 开发工作流（TDD）

### 详细步骤

```bash
# 同步 upstream 的 main 分支
git fetch upstream && git checkout main && git merge upstream/main

# 创建特性分支
git checkout -b feat/my-engine
# 或缺陷修复分支
git checkout -b fix/my-bugfix

# 1. Red：先写暴露缺陷/钉住新行为的失败测试
# 2. Green：写最少代码让测试通过
# 3. Refactor：不改行为地改进结构
# 4. 提交前质量门禁
cargo fmt
cargo clippy --all-features
cargo test --features full

# 完成后合并回 main（仓库主分支直行偏好，单人仓可直接推 main）
git checkout main
git merge feat/my-engine
# 解决冲突后重新运行质量检查，推送合并结果
git push upstream main
git branch -d feat/my-engine
```

### 测试要求

- 新增行为必须带钉住该行为的测试；修缺陷先写暴露缺陷的测试再修。
- **覆盖率门禁 ≥89%**：`cargo llvm-cov --features full` 度量，当前 89.44%。（曾自 documented ≥90%，后续功能提交令水位漂移；回升 90 前以实测为准，diff 覆盖率门禁仍为 ≥90% 不降标。）
- 金样输出变更（`tests/golden.rs`）必须在提交说明中写明变更理由。
- 测试两档都要过：`cargo test`（default 448）与 `cargo test --features full`（full 774）。

## 📐 代码规范

### Rust 最佳实践

- **DDD 三层**：领域类型与纯规则进 `domain/`（无网络/进程 IO；文件读写仅 `Input::to_bytes` 读盘与 `DirSink` 落盘两处例外）；端口与注册表进 `application/`；具体引擎进 `infrastructure/`。新引擎实现 `Converter` trait 后在 `infrastructure/engines.rs` 装配。
- **降级显性化**：引擎不可用返回 `Availability::Unavailable{reason}`，全链失败返回 `AllBackendsFailed{ext, attempts}`；禁止吞错、禁止用默认值掩盖失败、禁止把降级结果标成高质量。
- **availability 语义**：能降级产出的引擎恒返回 `Available`（把「能否产出」与「产出质量」分开），否则注册表会把它当不可用跳过。

### 命名约定

- 引擎 `name()` 用小写连字符（`latex-source`、`docx-deep`、`mineru`），与 `--engine` 和 `metadata.converter` 取值一致。
- 环境变量统一 `ALCHEMIZE_` 前缀（测试专用变量除外）。

### 代码质量要求

- **密钥脱敏**：任何含敏感字段的配置类型必须在 `Debug`/`Display`/serde 三通路脱敏并配单测（参照 `MineruConfig::api_token`——注意 derived Debug 不脱敏，需手工 impl）。
- 每次提交完整可工作（主分支直行偏好）：格式化零 diff、零新增 clippy 告警、双档测试全绿。

### 文档规范

- 公开项必须有 rustdoc（`//!`/`///`）。
- **版权头**：所有 `.rs` 文件（src/ 与 tests/）必须带两行版权头，位于第 1-2 行（`//!` 之前），第 3 行空行：

```rust
// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

```

- 许可证表达式与 `Cargo.toml` 的 `license` 一致（当前 Apache-2.0）；自其他仓库迁移的文件保留原版权年份区间（如实反映出处，如 `2025-2026`）。
- 文档版式沿用 confers 骨架（emoji 标题 + 📋 目录 + 相关文档收尾），新增文档按此模板。

## 📤 提交与 PR 流程

### 提交规范

Conventional Commits（与现有历史一致）：

```
feat(infra): T014 MinerU client family migrated (9 files, 41 tests)
test(real): real-world file parsing suite (11 fixtures)
docs: add 7-doc suite (architecture/API/user/FAQ/changelog/contributing/security)
```

- 类型：`feat` / `fix` / `refactor` / `test` / `docs` / `chore` / `perf` / `ci`。
- 一次提交一个聚焦变更；specmark 任务完成即提交（消息可带任务号）。
- 分批提交约定：deps → config → src → tests → docs → chore，每批前跑对应测试。

### PR 提交标准

- 提交信息符合 Conventional Commits。
- CI（若启用）全绿；本地门禁（fmt/clippy/双档测试）全绿。
- spec 修改（`specmark/specs/`）与行为变更同批提交。

### 评审标准

- 是否有钉住行为的测试；降级路径是否显性化（attempts/quality/metadata.converter）。
- 是否破坏能力域规格（`specmark/specs/`）——破坏即 block。

## 🤗 行为准则

参与本项目即同意保持专业与尊重：对事不对人，反馈聚焦代码与文档本身；骚扰、贬损性言论不可接受。详细条款沿用 Contributor Covenant 精神（本仓未内置单独文件）。

### 💝 感谢您为 Alchemize 做出贡献！
