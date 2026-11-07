# Alchemize

把任意文档炼成 Markdown（Rust，Apache-2.0）。

English version: [README_EN.md](README_EN.md)

## 支持的格式

| 族 | 格式 | 引擎 |
| --- | --- | --- |
| 文档 | pdf、docx/pptx/xlsx（+xls/ods）、odt/odp/rtf、旧版二进制 Office（doc/ppt/…，feature `anydoc`）、html/htm、epub、csv/tsv、txt/text/log/md、zip（递归）、ipynb、tex/latex（pandoc） | 注册表 fallback 链 |
| 代码/配置 | py/rs/js/ts/go/java/c/cpp/json/yaml/toml/xml/sh 等 ~45 扩展名 | 围栏代码块（语言标签映射） |
| 图片 | png/jpg/jpeg/gif/bmp/webp/tiff/svg | VisionProvider 或元数据降级 |
| 音频 | wav/mp3/ogg/flac/aac/m4a/wma/opus | vecboost 转录（feature `asr`）或元数据降级 |
| 视频 | mp4/avi/mkv/mov/wmv/flv/webm/m4v | ffmpeg 抽轨 → 音频链 |
| 字幕 | srt/vtt/ass/ssa | 原生解析（时间码列表） |

## 用法

```bash
# 转换（stdout）
alch convert paper.pdf

# 写文件 + 引擎偏好 + 资产模式
alch convert report.docx -o report.md --engine anydoc --assets dir|inline

# 引擎诊断 / 支持格式
alch doctor
alch formats
```

退出码：`0` 成功（含降级产出）；`1` 其他错误（输入不存在、写文件失败等）；`2` 不支持格式；`3` 全链失败（stderr 列出各引擎失败原因）。

## 📚 文档

| 文档 | 说明 |
|------|------|
| [🏗️ 架构文档](docs/ARCHITECTURE.md) | 设计原则、模块划分与 fallback 链机制 |
| [📘 API 参考](docs/API_REFERENCE.md) | 库公开 API、CLI 接口、环境变量 |
| [📖 用户指南](docs/USER_GUIDE.md) | 从安装到进阶的完整使用教程 |
| [❓ FAQ](docs/FAQ.md) | 常见问题解答 |
| [📋 更新日志](docs/CHANGELOG.md) | 版本变更记录 |
| [🤝 贡献指南](docs/CONTRIBUTING.md) | 开发环境、测试与提交约定 |
| [🔒 安全文档](docs/SECURITY.md) | 信任边界、SSRF 防护、漏洞报告 |

## 架构

单 crate DDD 三层：`domain`（类型与规则，无网络/进程 IO）→ `application`（Converter 注册表 priority fallback 链 + 端口）→ `infrastructure`（各格式转换器 + MinerU/ASR 后端）。

fallback 链（PDF 例）：anydoc（feature `anydoc`，`local_full`，priority 15）→ pdf-extract 本地文本 + TOC 剔除/结构启发（`local_fallback`，priority 10）→ MinerU 云端（feature `mineru` 且配置端点，`api`，priority 5）——本地优先，云端仅在本地链失败后兜底。转换结果的质量分层与实际引擎记录在 `metadata` 与 stderr 摘要行。

## Features

| feature | 引入 | 说明 |
| --- | --- | --- |
| `mineru` | MinerU 云端 PDF 引擎 | `ALCHEMIZE_MINERU_URL` 配置端点 |
| `anydoc` | office 全格式后端（firecrawl/anydoc, MIT） | 含旧版二进制 Office；图片字节按文档序导出 |
| `asr` | vecboost 转录 + Whisper 兼容 API | 自研 [vecboost](../vecboost)（Qwen3-ASR ONNX + silero VAD） |

`full` = 上述全部。默认零外部服务依赖。

## 转录（feature `asr`）

音频/视频转录调用自研 vecboost `transcribe` 门面（完全本地 ONNX 推理）：

- 模型 bundle：`ALCHEMIZE_TRANSCRIBE_MODELS` 指向目录，缺省探测 `~/.cache/alchemize/models/transcribe/`；布局为 `qwen3-conv_frontend.onnx`、`qwen3-encoder.int8.onnx`、`qwen3-decoder.int8.onnx`、`silero_vad.onnx`、`tokenizer/`（与 vecboost 契约一致）。缺失时引擎如实报 Unavailable 并附获取指引，不静默、不自动联网。
- 运行时动态库：`ORT_DYLIB_PATH` 指向 onnxruntime 动态库。
- 输出为 VAD 段级文本 + 时间戳；分段来自 silero VAD（语音活动检测），**非说话人识别**——输出不提供 SpeakerN 归属。
- Whisper 兼容云端后端仅在显式注入配置时激活。

发布顺序约束：vecboost 先于 alchemize 发布到 crates.io（当前为 path 依赖）。

## 许可证

Apache-2.0。致谢与边界：

- [firecrawl/anydoc](https://github.com/firecrawl/anydoc)（MIT）——office 格式后端依赖。
- [PDFMathTranslate](https://github.com/PDFMathTranslate/PDFMathTranslate)（AGPL-3.0）——**仅吸收设计思想（版面区域分类、图像页 OCR fallback、段落重组），零代码复制**，alchemize 保持 Apache-2.0。
- [mnemis](../mnemis)（自有）——转换器骨架迁移源。
