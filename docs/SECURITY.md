# 🔒 Alchemize 安全文档

Alchemize 的安全模型：**不可信输入、受限出口、密钥最小化**。本文档说明支持策略、信任边界、攻击面与对策、漏洞报告流程与最佳实践，内容逐项对应源码实现。

## 📋 目录

- [支持的版本与维护策略](#-支持的版本与维护策略)
- [漏洞报告流程](#-漏洞报告流程)
- [安全设计概览](#️-安全设计概览)
- [安全最佳实践](#-安全最佳实践)
- [安全联系方式](#-安全联系方式)
- [相关文档](#-相关文档)

## 📌 支持的版本与维护策略

| 版本 | 状态 | 支持到 |
| --- | --- | --- |
| 0.1.x | ✅ 维护中 | 0.2.0 发布前 |
| < 0.1.0 | ❌ 不支持 | — |

### 最低支持 Rust 版本（MSRV）

Rust **1.97.1**（`Cargo.toml` 的 `rust-version`，edition 2024；下限由 sdforge MSRV 决定）。安全修复不提高 MSRV。

### 依赖安全

```bash
cargo audit              # 运行安全审计
cargo update -d          # 更新漏洞通告数据库
```

关键依赖策略：ZIP 容器解析用 `zip` 9.x（RUSTSEC-2026-0367/0368/0369/0429 影响的 extract 系列路径不在使用面内，自锁 9.4.1）；TLS 走 rustls；`vecboost` 为同工作区 path 依赖（发布顺序 vecboost 先行）。

### 依赖审批流程

新增依赖需满足：默认特性关闭（`default-features = false` 优先）、无传递性网络栈（除非该依赖本身就是网络能力）、`cargo audit` 零新增通告、维护活跃度合理。依赖变更随 Cargo.lock 同批提交。

### 已知且可接受的风险

| 风险 | 现状 | 缓解 |
| --- | --- | --- |
| `with_max_file_size` 消费点 | 已在 facade 分发前拦截超限输入，并作为 ZIP 嵌套条目上限 | 覆盖注册表分发与 zip 分支；tar/tgz/7z 嵌套固定 64 MiB/条目 + 256 MiB/整包，gz/xz/zst 单流解压上限 64 MiB，均不受此值影响；MinerU precision 另有 200MB 上限 |
| libsqlite3-sys 0.35 存量通告 RUSTSEC-2026-0459（C SQLite CVE-2026-119375，medium） | `data` feature 传递依赖，存量非新增 | cargo-audit 已核；升级 rusqlite 属独立依赖变更，随下一依赖批次处理 |
| 仓库无 CI 与 git 钩子 | 门禁靠贡献者本地执行 | CONTRIBUTING 明确列出本地门禁清单 |

### 供应链与门禁

- `Cargo.lock` 入库锁定依赖图；依赖升级需过 `cargo audit`。
- 全部源文件带 `SPDX-License-Identifier: Apache-2.0` 两行头（59/59），license 扫描工具可机器识别文件级权属。
- specmark 规格目录不入库（gitignore），主 specs 作为本地规格真源。
- CLI/MCP 封装框架 sdforge（MIT，crates.io 版本钉 0.5.0-rc.10）与 rmcp（`mcp` feature 独占）经 `cargo audit` 一并纳管。
- html-deep 引擎（feature `html-deep`）为本仓自研迭代状态机（Apache-2.0，差分验证对齐记录见仓库历史），依赖 scraper 0.29、dom_smoothie 0.19、icu_properties 2.3（均在依赖树内）；`--base-url` 仅在显式提供时改写链接。
- lopdf 0.47（crates.io 直连，MIT）：解压炸弹防护在调用点显式设限——文档解析与逐页文本抽取每页/每流 64 MiB 上限（`LoadOptions::with_max_decompressed_size` + `extract_text_chunks_with_limit`）、图片流 `decompressed_content_with_limit` 64 MiB，另有 256 MiB 全文档累计文本预算（拒绝时峰值 ≈ 预算 + 单页 64 MiB）；超限以 `AlchemizeError::Pdf` 显性拒绝。炸弹类错误（`MemoryLimitExceeded`）由 lopdf 原生传播、无吞错路径；损坏流（非炸弹类解码错误）回退原始压缩字节产出乱码文本——数据完整性由产物质量层兜底，不构成内存风险。

## 🐛 漏洞报告流程

### 如何报告

**不要**通过公开 GitHub Issue 报告安全漏洞。使用 GitHub 私密安全通告：仓库 **Security 标签页 → Report a vulnerability**。

### 报告内容

- 构造输入文件（注意脱敏）或命令行复现步骤
- `alch --version` / `alchemize::VERSION` 与编译特性
- 影响评估：泄露/拒绝服务/绕过面

### ⏱️ 响应承诺

- **初评**：14 天内确认收到并给出初评
- **修复**：按严重程度排期（CRITICAL 立即热修，HIGH 随下一补丁版，MEDIUM/LOW 随常规版本）
- **发布**：修复随新版本发布并附通告，通告中致谢报告者（除非要求匿名）

### 严重程度分级

| 级别 | 判据 |
| --- | --- |
| CRITICAL | 远端代码执行、SSRF 打穿白名单、凭据泄漏到日志/序列化 |
| HIGH | 本地解析器内存破坏、降级静默吞错、权限边界绕过 |
| MEDIUM | 拒绝服务（资源耗尽）、错误信息泄露敏感路径 |
| LOW | 文档失实、非敏感信息泄漏 |

### 漏洞披露时间线

修复发布后才披露细节；通告同步更新本页「已知且可接受的风险」表。

## 🛡️ 安全设计概览

### 输入解析安全

文档输入**完全不可信**（邮件附件/下载页/爬虫来源）。解析面与对策：

| 攻击面 | 解析器 | 对策 |
| --- | --- | --- |
| zip / docx / pptx / epub（ZIP 容器） | `zip` crate（deflate） | 声明式依赖 + Cargo.lock 锁定；解析失败返回 `Zip`/`Xml` 结构化错误而非 panic |
| pdf | `lopdf` 0.47（crates.io 直连；pdfium 档另见下行） | 解析失败返回 `Pdf` 错误；TOC 剔除与结构启发只影响产出质量；每页/每流解压上限 64 MiB + 256 MiB 全文档累计文本预算，超限显性拒绝，防解压炸弹；lopdf 原生传播解压错误（无吞错路径） |
| xml | `quick-xml` | 无外部实体解析路径 |
| tex/latex | pandoc 子进程 | 仅 PATH 探测 + 参数化调用，无 shell 拼接；围栏兜底路径无子进程 |
| 构造输入的资源消耗 | 各解析器 | zip 门面拒绝嵌套递归（无界深度）；HTML 引擎全程迭代遍历（DOM 深度不进调用栈）；响应读取内存有界 |

### SSRF 防护

全部网络出口集中在 MinerU 后端（feature `mineru`），下载 URL 必须通过 `ValidatedFileUrl` 校验（`validated_url.rs`）。MCP 面（`alch-mcp`）走 stdio 传输——**传输面无网络出口**（convert 工具在 `mineru` 特性与端点环境变量齐备时可出网，属业务能力而非传输）；HTTP/gRPC/WS 协议未启用：

- 仅允许 HTTPS scheme；
- 拒绝环回（`127.0.0.0/8`、`::1`、`localhost`）、`0.0.0.0/8`、广播地址；
- 拒绝 RFC1918 私网（`10/8`、`172.16/12`、`192.168/16`）、CGNAT（`100.64/10`，含云厂商元数据端点 `100.100.100.200`）、链路本地（`169.254/16`）；
- IPv4 映射/兼容的 IPv6（`::ffff:a.b.c.d` 等）按内嵌 IPv4 同样分类，防绕过；
- 域名白名单默认 `mineru.net` / `cdn.mineru.net` / `api.mineru.net`（`DEFAULT_MINERU_ALLOWLIST`）；
- 违规返回 `AlchemizeError::SecurityViolation { kind, message }`，kind 见 `SecurityViolationType`（含 `MissingSignature` 预签名缺失）。

### 响应体积限制

远端响应读取全部经限量读取器（`http_limit.rs`），按用途分级：

| 响应类型 | 上限 |
| --- | --- |
| 提交应答 | 64 KB |
| 错误应答 | 8 KB |
| 轮询应答 | 50 MB |
| 结果应答 | 50 MB |

预分配上限 8 MB（Content-Length 声明再大也只预分配 8MB，超限流式拒绝）；超限返回 `AlchemizeError::ResponseTooLarge { actual, limit }`。

### 凭据与敏感信息处理

- **凭据来源与传递**：`MINERU_API_TOKEN` 或 `MineruConfig::api_token`（两者均转成 Bearer token，仅发往通过 `validate_base_url` 的端点）；无内置密钥、无加密存储需求。token 会随请求发往配置的 base_url：`validate_base_url` 约束主机与 scheme 白名单，但本地部署使用明文 http 端点时由部署方自行承担链路风险。容器嵌套另有链级 512 MiB 累计解压预算（按转换链计数）；每层在解压物化后才计费，瞬时峰值可多出一层的 256 MiB。
- **三通路脱敏**：`MineruConfig::api_token` 在 `Debug`（手工 impl）、`Display`、serde 序列化（`skip_serializing`）中均不出现明文——单测 `mineru_config_display_redacts_token` 钉住三条通路。
- **日志不落密钥**：tracing 输出仅含引擎名、优先级、失败原因等诊断字段。

### 不自动联网原则

- ASR 模型 bundle 缺失：报 `Unavailable` + 获取指引，**不自动下载**——模型获取是用户的完整性与供应链决策。
- Whisper 兼容云端后端：仅显式注入配置时激活，绝不默认启用。
- 视觉模型：无内置 provider，需库使用者显式注入（无隐式云兜底）。

## ✅ 安全最佳实践

### 面向库使用者

- 不可信环境用 `AlchemizeBuilder::new().with_no_remote().build()`——进程零网络出口。
- 处理不可信文件时优先隔离环境（容器/沙箱），本工具的解析面不含网络执行路径，但解析器自身缺陷深度防御仍依赖隔离。
- 打印含敏感字段的配置前确认其 Debug/Display 已脱敏；自定义类型含密钥字段时效仿 `MineruConfig` 手工实现。

### 生产环境配置示例

```bash
# 启用全功能（云端 PDF + office 全格式 + 本地 ASR）
cargo build --release --features full

# 生产环境必填（precision 云端令牌）
export MINERU_API_TOKEN="<token>"

# 离线/合规环境：不设 ALCHEMIZE_MINERU_URL，MinerU 引擎不注册
alch convert --no-remote report.docx -o report.md
```

### 环境变量

| 变量 | 说明 |
| --- | --- |
| `MINERU_API_TOKEN` | 生产环境用云端 Precision API 时必填（bearer token） |
| `ALCHEMIZE_MINERU_URL` | 设置后注册 MinerU 引擎；不设置即无云端出口 |
| `ALCHEMIZE_TRANSCRIBE_MODELS` / `ORT_DYLIB_PATH` | 本地 ASR 路径，无网络含义 |

### 安全加固清单

- [ ] 不可信环境 `--no-remote` / 不设 `ALCHEMIZE_MINERU_URL`
- [ ] `cargo audit` 纳入升级流程
- [ ] 凭据走 secret 管理器注入环境变量，不进 shell 历史
- [ ] 升级时复核本页「已知且可接受的风险」表
- [ ] 自定义引擎不引入新的网络出口；确需时复用 `ValidatedFileUrl` 校验

## 📎 安全联系方式

- 漏洞报告：GitHub 私密安全通告（**Security → Report a vulnerability**），见上文流程
- 一般问题：GitHub Issue（非安全缺陷）

## 📚 相关文档

- [🏗️ 架构文档](ARCHITECTURE.md) — 安全设计在架构中的位置
- [📖 用户指南](USER_GUIDE.md) — 离线与隐私实践
- [🤝 贡献指南](CONTRIBUTING.md) — 贡献与提交流程
- [❓ FAQ](FAQ.md) — 「Alchemize 安全吗？」速答
