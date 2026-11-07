<!-- domain: code -->

# migrate-mnemis-parser

## Motivation

alchemize（"Transmute any document into Markdown"）当前是 0.0.0 占位包：仅 `Format` 枚举骨架，无任何转换后端。mnemis 仓库的 `mnemis-parser` crate（~6600 行，17 个转换器 + priority fallback 注册表 + 格式检测）已实现大部分目标能力，但存在三类问题：① 与 `mnemis-core`/`mnemis-model` 耦合（7 文件引用 ParserConfig/MineruParserConfig/VisionProvider），无法独立复用；② `converters/anydoc.rs` 是孤儿文件（无 `mod anydoc` 声明、builder 不注册、模式匹配与现行 `types.rs` 不兼容，从未参与编译）；③ PDF/DOCX 转换为纯文本提取，无图片资产、无公式 LaTeX、无版面保真——正是"图片插入位置不变、LaTeX 正常、引用/代码正常渲染"需求的最大缺口。

本变更把 mnemis-parser 骨架解耦迁移至 alchemize，并吸收三路外部能力补齐保真短板：liuxiang/cangjie skill 脚本的转换逻辑（五级 PDF 引擎链、LaTeXML LaTeX 提取、ffmpeg 抽轨 + ASR）、firecrawl/anydoc crate（MIT，14 格式→GFM，OMML/MathML→LaTeX，图片字节保留）、PDFMathTranslate 的设计模式（版面区域分类、图像页 OCR fallback、段落重组）。

## Scope

- **骨架迁移（解耦）**：mnemis-parser 的 domain 类型（Input/InputFormat/ConvertResult/ConversionQuality/ContentType）、`Converter` trait、`ConverterRegistry`（priority 排序 + fallback 链 + `convert_pref`）、`ParserBuilder`/`Parser`、格式检测（magic bytes → 扩展名）全部迁入 alchemize；`ParserConfig` 耦合替换为 alchemize 自有 `MineruConfig`；两个 `VisionProvider` trait（types.rs 与 mnemis_model）合并为一个 port
- **文档族转换器迁移**：text/csv/zip/html(htmd)/epub(rbook)/xlsx(calamine)/docx+pptx(quick-xml)/ipynb/pdf(pdf-extract)，各带迁移测试
- **anydoc 集成重写**：修复孤儿文件问题，按现行类型系统重写 `AnydocConverter`（feature `anydoc`，catch_unwind + spawn_blocking），office 格式（docx/odt/rtf/epub/pptx/xlsx/csv/pdf）走 anydoc 优先、自研解析器作 fallback
- **PDF 引擎 fallback 链（吸收 liuxiang pdf2md 五级链模式）**：MinerU 云端（feature `mineru`，迁移现成 client：wrapper/agent/precision/http_limit/validated_url）→ anydoc PDF（文本型）→ pdf-extract 纯文本；质量分级 Api/LocalFull/LocalFallback 随链路标注
- **资产管线**：`AssetSink` 抽象（目录模式：图片落 `images/` 相对路径引用；单文件模式：base64 内嵌），MinerU zip 图片解包、anydoc 图片字节、按文档顺序原位插入（顺序即位置）
- **LaTeX/引用/代码保真**：html 转换前置预处理（`<math alttext>`/`annotation` → `$...$`，吸收 liuxiang html2md `_get_latex`）；`LatexSourceConverter`（.tex 源码经 pandoc 子进程转换 + 数学归一化）；`latex_validate` 模块（吸收 check_latex 核心规则：数学定界符配对、环境闭合）
- **媒体族迁移**：image（VisionProvider 可用→描述+OCR，不可用→元数据降级）、subtitle（srt/vtt）、timeline、audio、video（AudioOnly 默认）、dedup
- **ASR 承载（调用自研 vecboost，完全 Rust）**：`TranscribeProvider` port + 三个后端——`VecboostTranscriber`（依赖 vecboost `transcribe` 门面：silero VAD 分段 + Qwen3-ASR 段级转录，16kHz mono wav 输入契约，非 wav 格式经 ffmpeg 预转；模型 bundle 与 ORT 动态库路径显性探测）、`WhisperApiTranscriber`（Whisper 兼容 API）、元数据降级；输出为 VAD 段级文本+精确时间戳（无 SpeakerN 标签，disclaimer 注明 VAD 分段非说话人识别）
- **外部工具端口**：pandoc/ffmpeg/ASR 统一 `ExternalTool` 探测（`which` 探测 + 超时），不可用即 `Availability::Unavailable{reason}`，由注册表跳过或降级
- **CLI**：`alch <input> [-o out.md] [--engine NAME] [--assets dir|inline] [--no-remote]` + `alch doctor`（各引擎 availability 报告）
- **质量工程**：golden fixture 语料（每格式 ≥1 例）、proptest（魔数检测模糊）、覆盖率 ≥90%、`alch doctor` 端到端测试
- **覆盖度增补（四轮拍板 2026-11-07，覆盖度审计后纳入）**：
  1. anydoc 完整扩展名表启用（14 格式：doc/docx/docm/ppt/pps/pot/pptx/pptm/ppsx/ppsm/xls/xlsx/xlsm/xlsb/odt/ods/odp/rtf/epub/csv/pdf）+ ODF zip 魔数检测（`content.xml`/`mimetype` 条目特征）
  2. 代码/配置文件族：扩 TextConverter 白名单（py/rs/js/ts/go/java/c/cpp/h/py/json/yaml/yml/toml/xml/sh/bash 等）→ 按扩展名映射语言标签的围栏代码块输出（```python 等；扩白名单不新建转换器）
  3. tsv：csv 转换器加扩展名 + 分隔符探测（tab 计数 vs 逗号）
  4. detect_magic_bytes 盲区补齐：GIF8 / RIFF+WEBP / OggS / fLaC / ISOBMFF ftyp（mp4/mov/m4a major brand）/ RIFF+AVI
  5. ass/ssa 字幕：ffmpeg 转换通道（`ffmpeg -i x.ass -f srt` → 复用 srt 转换器；ffmpeg 缺失该路径 Unavailable）

## Non-Goals

- **不复制 PDFMathTranslate 任何代码**：其许可证为 AGPL-3.0，alchemize 保持 Apache-2.0；只吸收版面区域分类、图表公式区域不动、图像页 OCR fallback、段落重组四个设计模式（思想层）
- **不修改 mnemis 仓库任何文件**：mnemis-parser 保持原样，双轨并存；切换依赖是独立后续变更（见 Deferred）
- **不做说话人标签（SpeakerN）**：vecboost 的 silero VAD 分段无说话人身份信息，cangjie 能量法标签是近似分段（原文自带 disclaimer「非真实声纹识别」）——alchemize 输出 VAD 段级文本+时间戳，不伪造说话人归属。重开条件：vecboost 未来提供 speaker-diarization 能力
- **不做 Qwen3-ASR 模型的移植**：vecboost 已完成 Rust 原生实现（ONNX 三图 + ort + silero VAD，见 `vecboost/src/transcribe/` 门面与 `transcribe-poc/CONTRACT.md`）——alchemize 仅作消费方集成，不重复造轮子
  - **↻ 注记**：本条替代二轮的「不做 Qwen3-ASR 移植（torch 无 Rust 等价）」——该理由经三轮拍板（vecboost 既有实现）作废，语义从「不可移植」变为「无需移植」
- **不实现视频 GPU 帧分析**：保持 mnemis-parser 的 `VideoProcessingMode::AudioOnly` 默认，`Gpu` 模式仅保留枚举与接口占位
- **不集成本地 OCR 引擎**（tesseract 绑定等）：图像理解走 `VisionProvider` port（外部实现注入），与 mnemis-parser 现状一致
- **不迁移 liuxiang/cangjie 的非转换脚本**（search_papers/citation_graph/passport_tool/skill_lint/分镜/去AI痕迹等）：它们与"文件→Markdown"无关
- **低频格式不做**（四轮审计结论）：eml/msg（邮件）、mobi/azw3（旧电子书）、pages/numbers/keynote（Apple 专有）、one（OneNote）、7z/rar（zip 之外的压缩包）、djvu/ps/eps——重开条件：出现对应格式的实际转换需求

## Deferred

- **mnemis-parser 切换依赖 alchemize**：原因——alchemize 未达 0.1.0 稳定前不应拖累 mnemis 主项目；重开条件——alchemize 0.1.0 发布且 golden 语料证明覆盖 mnemis-parser 全部现有能力（含 MinerU 与媒体族）
- **视频 `Gpu` 帧分析**（VisionProvider 逐帧→markdown 时间线）：原因——无 GPU 环境验证渠道；重开条件——需要视频画面理解的实际消费场景出现
- ~~Rust 原生 OCR/ASR 后端~~：**2026-11-07 重开**——ASR 部分经用户拍板转入 Scope（见 Clarifications 二轮修订与 Non-Goals 翻转注记）；OCR 部分维持推迟（原因：模型分发成本；重开条件：无网络离线部署需求）
- **MinerU precision 通路 result_url 的 zip 下载与资产解包**：原因——迁移源两通路均只取 markdown 字符串（agent 直返、precision 仅解析状态），zip 解包属新功能且无 mock 端点可当轮验证；alchemize 资产管线（T013）已就绪，解包结果可直接挂 ConvertResult.assets。重开条件：出现 precision 通路消费场景或获得可 mock 的 result_url 端点
- **docx 自研解析器的图片原位插入升级**（quick-xml 逐 drawing anchor 定位）：原因——anydoc 已按文档模型保序保留图片字节，自研 docx 转换器仅作 fallback（文本保真即可）；重开条件——anydoc 对某类 docx 的图片位置还原被实测证明不可接受

## Clarifications

用户未回复 AskUserQuestion（2026-11-07），以下四项按推荐默认值落定，**均可否决后重议**（走 revision-protocol）：

- **[scope]** 架构形态：单 crate 内 DDD 三层（domain/application/infrastructure + cli bin），与 mnemis-parser 同构，迁移摩擦最小，crates.io 单包发布
- **[license]** PDFMathTranslate 吸收方式：仅设计模式（思想层），零代码复制，保持 Apache-2.0
- **[technical]** ASR 承载：`TranscribeProvider` trait + Python 子进程默认后端 + Whisper API 云端后端 + 双不可用降级
  - **↻ 2026-11-07 二轮修订**：完全 Rust（whisper-rs + 能量法重实现）——已被三轮覆盖，见下
  - **↻ 2026-11-07 三轮修订，用户拍板（"语音的转录改为使用 vecboost 库，那里有完整实现"）**：转录改调自研 **vecboost 0.3.0-rc.3** 的 `transcribe` 门面（`open(models_dir, opts)` → `detect_segments` silero VAD 真分段 + `transcribe_segment` 段级转录，Qwen3-ASR ONNX 三图 + tokenizer，feature 门控）——**完全 Rust 且上轮申报的硬外部阻塞「Qwen3-ASR 无 Rust 等价推理」就此作废**（vecboost 已完成该实现）；cangjie 能量法说话人近似分段**退役**（vecboost VAD 分段在精度上优于能量近似，且无说话人身份信息——SpeakerN 标签功能随之移除，输出为段级文本+精确时间戳）；模型 bundle 目录与 ORT_DYLIB_PATH 经环境变量/探测显性化；Whisper API 后端与元数据降级保留
- **[scope]** mnemis-parser 去向：独立孵化，mnemis 仓库零改动，切换列入 Deferred

## 来源与验证

| 来源 | 用途 | 验证状态 |
| ---- | ---- | -------- |
| `mnemis/crates/mnemis-parser/src/**`（40 文件，6567 行） | 骨架与转换器迁移源 | verified：已逐模块读取，Cargo.toml/类型/注册表/构建器均为一手源码 |
| `mnemis/crates/mnemis-core/src/config/types.rs:1412-1436` | MineruParserConfig/ParserConfig 耦合面 | verified：grep 定位，字段 3 URL；wrapper 另用 api_token/timeout_secs |
| `liuxiang/scripts/pdf2md.py`（382 行） | 五级 PDF 引擎链、图片按页抽取、TOC 剔除、段落重组 | verified：函数清单已读 |
| `liuxiang/scripts/html2md.py`（521 行） | LaTeXML 树遍历、`_get_latex`（alttext/annotation）、图表/公式/参考文献转换 | verified：类与方法清单已读 |
| `liuxiang/scripts/download_paper.py:599-698` | pandoc 数学归一化、LaTeX 噪声剔除、latex_source_to_markdown | verified：函数签名已读 |
| `liuxiang/scripts/check_latex.py`（332 行） | LaTeX 质量校验规则 | unconfirmed：未逐行读，迁移任务 T014 需先读全文 |
| `cangjie/scripts/{extract_audio,cangjie}.py` | ffmpeg 抽轨、统一入口编排 | verified：入口与主函数已读 |
| `vecboost/src/transcribe/{mod,facade}.rs`（0.3.0-rc.3，Apache-2.0） | 转录门面：`open(models_dir,&TranscribeOpts)→TranscribeSession`、`detect_segments`（silero VAD）、`transcribe_segment`（段级）、`read_wav_16k_mono`（16kHz mono 无重采样契约）；模型 bundle=Qwen3-ASR ONNX 三图+VAD+tokenizer | verified：2026-11-07 三轮修订时源码已读；ORT 动态库 load-dynamic、段级并行约束（rc.13 禁并发 Run）已确认 |
| `vecboost/transcribe-poc/CONTRACT.md` | CLI 契约/模型目录布局/ORT_DYLIB_PATH 运行时依赖 | verified：全文已读 |
| `cangjie/scripts/transcribe_diarize_fw.py`（faster-whisper + 能量法分段） | 输出格式契约（时间戳/语言/总时长字段）；能量法算法已**退役**（vecboost silero VAD 替代） | verified：二轮全文已读 |
| `anydoc 0.2`（crates.io，MIT，firecrawl/anydoc） | office→GFM 依赖 | verified：mnemis-parser Cargo.toml 已依赖；GitHub 页确认 14 格式/图片字节/OMML→LaTeX |
| `PDFMathTranslate`（GitHub，AGPL-3.0，38.3k star） | 设计模式参考 | verified：README/架构已读，确认无独立提取模块、许可证 AGPL |

- **[scope]** 覆盖度增补（2026-11-07 四轮，AskUserQuestion 全选）：旧版/宏 Office+ODF（anydoc 完整表）、代码/配置族（扩 TextConverter 白名单+围栏语言标签——用户确认代码文件本就是文本、实现即扩白名单）、tsv+魔数盲区、ass/ssa 字幕（ffmpeg 通道）；低频格式（eml/msg/mobi/azw3/pages/numbers/keynote/one/7z/rar/djvu/ps）入 Non-Goals

## NEEDS CLARIFICATION

（无——关键决策已全部落定，任务可直接执行）
