# 🤝 Alchemize 贡献指南

感谢关注 alchemize。本文档说明开发环境、测试与质量门禁、代码规范与提交约定。

## 📋 目录

- [开发环境](#开发环境)
- [测试与覆盖率](#测试与覆盖率)
- [代码规范](#代码规范)
- [提交约定](#提交约定)
- [变更流程](#变更流程)
- [报告问题](#报告问题)

## 开发环境

- **Rust 1.91+**（`Cargo.toml` 的 `rust-version`，edition 2024）。
- 可选外部依赖：`ffmpeg`（视频抽轨，PATH 探测）、`pandoc`（LaTeX 高保真路径，缺失时围栏兜底）。
- ASR 本地推理需 onnxruntime 动态库与模型 bundle（见[用户指南](USER_GUIDE.md#音视频转录feature-asr)）。

```bash
git clone https://github.com/Kirky-X/alchemize.git
cd alchemize
cargo build                 # 默认特性
cargo build --features full # 全功能（含 vecboost path 依赖）
```

注意：`full` 特性依赖同工作区兄弟仓 [vecboost](https://github.com/Kirky-X/vecboost)（path 依赖 `../vecboost`）；发布顺序 vecboost 先于 alchemize。

## 测试与覆盖率

两档测试，提交前至少跑满其中一档：

```bash
cargo test                    # default 档（317 例，无外部服务依赖）
cargo test --features full    # full 档（396 例）
cargo clippy --all-features   # 零新增告警
cargo fmt --check
```

- **覆盖率门禁 ≥90%**（本仓真源约定；当前 90.49%）。度量：`cargo llvm-cov --features full`。
- 新增行为必须带钉住该行为的测试；修缺陷先写暴露缺陷的测试再修。
- 金样输出变更（`tests/golden.rs`）必须在提交说明中写明变更理由。

## 代码规范

- **DDD 三层**：领域类型与纯规则进 `domain/`（零 IO）；端口与注册表进 `application/`；具体引擎进 `infrastructure/`。新引擎实现 `Converter` trait 后在 `infrastructure/engines.rs` 装配。
- **降级显性化**：引擎不可用返回 `Availability::Unavailable{reason}`，全链失败返回 `AllBackendsFailed{ext, attempts}`；禁止吞错、禁止用默认值掩盖失败、禁止把降级结果标成高质量。
- **密钥脱敏**：任何含敏感字段的配置类型必须在 `Debug`/`Display`/serde 三通路脱敏并配单测（参照 `MineruConfig::api_token`）。
- ** rustdoc**：公开项必须有 `//!`/`///` 文档注释；示例代码保持可编译。

## 版权头规范

所有 `.rs` 文件（src/ 与 tests/）必须带两行版权头，位于文件第 1-2 行（`//!` 模块文档之前），第 3 行空行：

```rust
// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

```

- 许可证表达式与 `Cargo.toml` 的 `license` 一致（当前 Apache-2.0）。
- 自其他仓库迁移的文件保留原版权年份区间（如实反映出处，如 `2025-2026`）。
- 新文件提交前补头；SPDX 头是 license 扫描工具识别文件级许可的依据。

## 提交约定

Conventional Commits（与现有历史一致）：

```
feat(infra): T014 MinerU client family migrated (9 files, 41 tests)
test(real): real-world file parsing suite (11 fixtures)
docs: T024 README rewrite — format matrix, fallback chains
```

- 类型：`feat` / `fix` / `test` / `docs` / `refactor` / `chore`。
- 一次提交一个聚焦变更；specmark 任务完成即提交（消息可带任务号）。

## 变更流程

本仓库采用 specmark 规格驱动流程：

1. `/specmark propose <name>` 生成 proposal/design/tasks（长程变更含能力域 delta specs）。
2. `/specmark apply` 按 tasks 顺序实施，逐项勾选，禁止跳过。
3. `/specmark converge` 对账交付物与规格；`/specmark archive` 归档。
4. 规格文件在 `specmark/`（归档时随仓库保留，作为后续变更的事实源）。

## 报告问题

- 缺陷：开 GitHub Issue，附最小复现文件（注意脱敏）、命令、期望与实际输出。
- 安全问题：**不要**开公开 Issue，按[安全文档](SECURITY.md)的私密渠道报告。
