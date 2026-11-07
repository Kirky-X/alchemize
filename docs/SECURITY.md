# 🔒 Alchemize 安全文档

Alchemize 的安全模型：**不可信输入、受限出口、密钥最小化**。本文档说明信任边界、攻击面与对策、凭据处理和漏洞报告流程，内容逐项对应源码实现。

## 📋 目录

- [信任边界](#信任边界)
- [本地解析攻击面](#本地解析攻击面)
- [网络出口与 SSRF 防护](#网络出口与-ssrf-防护)
- [响应体积限制](#响应体积限制)
- [凭据与敏感信息处理](#凭据与敏感信息处理)
- [不自动联网原则](#不自动联网原则)
- [漏洞报告](#漏洞报告)

## 信任边界

```
不可信区                    信任边界                 受控区
────────────────────────────────────────────────────────────
用户提供的文档文件    →    本地解析器             →   markdown 产出
（zip/epub/docx/…）        （内存配额内解析）
 MineralsU/Whisper 端点 ←   显式配置的 URL+令牌     ←   用户环境变量
```

- 文档输入**完全不可信**：可能来自邮件附件、下载页、爬虫，格式构造可含恶意载荷。
- 网络出口**默认不存在**：默认特性构建不建立任何外部连接；云端能力需显式配置（环境变量/库 API 注入）才激活。
- `--no-remote` 或未配置端点时，进程零网络出口。

## 本地解析攻击面

| 攻击面 | 解析器 | 对策 |
| --- | --- | --- |
| zip / docx / pptx / xlsx / epub（ZIP 容器） | `zip` crate（deflate） | 声明式依赖 + 版本锁定（`Cargo.lock`）；解析失败返回 `Zip`/`Xml` 错误而非 panic |
| pdf | `pdf-extract` | 解析失败返回 `Pdf` 错误；TOC 剔除与结构启发只影响产出质量，不扩大权限 |
| xml | `quick-xml`（serialize 特性） | 无外部实体解析路径 |
| tex/latex | pandoc 子进程 | `tools/pandoc.rs` 仅 PATH 探测 + 参数化调用，无 shell 拼接；围栏兜底路径无子进程 |
| 构造输入的资源消耗 | 各解析器 | zip 门面拒绝嵌套递归（无界深度）；builder 预留 `with_max_file_size`（注：当前引擎装配未消费该值，见 [API 参考](API_REFERENCE.md)） |

任何解析缺陷请按[漏洞报告](#漏洞报告)流程提交。

## 网络出口与 SSRF 防护

全部网络出口集中在 MinerU 后端（feature `mineru`）：

1. **出口清单**：`MineruConfig` 的 `base_url`（本地服务，默认 `http://localhost:8888`）、`agent_base_url` 与 `precision_base_url`（云端，默认 `https://mineru.net`）。此外无任何网络调用。
2. **ValidatedFileUrl 校验**（`infrastructure/remote/mineru/validated_url.rs`）——MinerU 返回的文件下载 URL 在使用前必须通过：
   - 仅允许 HTTPS scheme；
   - 拒绝环回地址（`127.0.0.0/8`、`::1`、`localhost`）、`0.0.0.0/8`、广播地址；
   - 拒绝 RFC1918 私网（`10/8`、`172.16/12`、`192.168/16`）、CGNAT（`100.64/10`，含云厂商元数据端点 `100.100.100.200`）、链路本地（`169.254/16`）；
   - IPv4 映射/兼容的 IPv6（`::ffff:a.b.c.d` 等）按内嵌 IPv4 同样分类，防绕过；
   - 域名白名单默认 `mineru.net` / `cdn.mineru.net` / `api.mineru.net`（`DEFAULT_MINERU_ALLOWLIST`）。
   - 违规返回 `AlchemizeError::SecurityViolation { kind, message }`，kind 见 `SecurityViolationType`（含 `MissingSignature` 预签名缺失）。
3. **无重定向跟随的隐式放行**：下载 URL 逐次校验，白名单外域名一律拒绝。

## 响应体积限制

远端响应读取全部经 `http_limit.rs` 的限量读取器（防恶意/异常响应撑爆内存），按用途分级：

| 响应类型 | 上限 |
| --- | --- |
| 提交应答 | 64 KB |
| 错误应答 | 8 KB |
| 轮询应答 | 50 MB |
| 结果应答 | 50 MB |

预分配上限 8 MB（Content-Length 声明再大也只预分配 8MB，超限流式拒绝）。超限返回 `AlchemizeError::ResponseTooLarge { actual, limit }`。

## 凭据与敏感信息处理

- **凭据只经环境变量传递**：`MINERU_API_TOKEN`（Precision API bearer token）；无配置文件、无内置密钥、无加密存储需求。
- **三通路脱敏**：`MineruConfig::api_token` 在 `Debug` 派生、`Display` 实现、serde 序列化（`skip_serializing`）中均不出现明文——由单测 `mineru_config_display_redacts_token` 钉住，防止日志/序列化泄漏。
- **日志不落密钥**：tracing 输出仅含引擎名、优先级、失败原因等诊断字段。

## 不自动联网原则

- ASR 模型 bundle 缺失：报 `Unavailable` + 获取指引，**不自动下载**——模型获取是用户的完整性与供应链决策。
- Whisper 兼容云端后端：仅显式注入配置时激活，绝不默认启用。
- 视觉模型：无内置 provider，需库使用者显式注入（无隐式云兜底）。

## 漏洞报告

**不要**通过公开 GitHub Issue 报告安全漏洞。

1. 使用 GitHub 私密安全通告（仓库 Security 标签页 → Report a vulnerability）。
2. 附最小复现：构造文件、命令行、版本（`alch --version` 或 `alchemize::VERSION`）、影响评估。
3. 修复将按常规版本节奏发布，通告中致谢报告者（除非要求匿名）。

支持版本：最新 release 与 main 分支。
