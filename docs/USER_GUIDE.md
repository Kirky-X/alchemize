# 📖 Alchemize 用户指南

从安装到进阶的完整使用教程。命令与行为均以当前源码为准；API 细节见 [API 参考](API_REFERENCE.md)，设计背景见[架构文档](ARCHITECTURE.md)。

## 📋 目录

- [安装](#安装)
- [快速开始](#快速开始)
- [支持的格式](#支持的格式)
- [输出与资产](#输出与资产)
- [引擎选择与诊断](#引擎选择与诊断)
- [PDF 高保真：MinerU 配置](#pdf-高保真mineru-配置)
- [音视频转录（feature `asr`）](#音视频转录feature-asr)
- [离线与隐私](#离线与隐私)
- [故障排查](#故障排查)

## 安装

要求 Rust 1.91+（见 `Cargo.toml` 的 `rust-version`）。

```bash
# 从源码构建（默认特性：零外部服务依赖）
cargo build --release
# 二进制位于 target/release/alch

# 全功能（MinerU 云端 + anydoc office 后端 + 本地 ASR）
cargo build --release --features full

# 安装到本地
cargo install --path . --features full
```

特性说明：`mineru`（云端 PDF）、`anydoc`（office 全格式，含旧版二进制 Office）、`asr`（本地转录）、`full`（全部）。未编译的特性对应引擎不会出现，行为上表现为「该格式走其余引擎或报不支持」。

## 快速开始

```bash
# 转换到 stdout
alch convert paper.pdf

# 写文件（父目录自动创建），stderr 给摘要行
alch convert report.docx -o report.md
# converted report.docx -> report.md [local_full] (engine: anydoc)
```

摘要行 `[quality]` 是质量分层：`api`（云端高保真）、`local_full`（本地全功能）、`local_fallback`（本地降级，如纯文本提取）。降级产出同样算成功（退出码 0）——结果是否可接受由质量标签判断，不吞不瞒。

## 支持的格式

| 族 | 格式 | 说明 |
| --- | --- | --- |
| 文档 | pdf、docx/pptx/xlsx（+xls/ods）、odt/odp/rtf、旧版二进制 Office（doc/ppt/…，需 `anydoc`）、html/htm、epub、csv/tsv、txt/text/log/md、zip（递归）、ipynb、tex/latex（pandoc 探测，缺失时围栏兜底） | 注册表 fallback 链 |
| 代码/配置 | py/rs/js/ts/go/java/c/cpp/json/yaml/toml/xml/sh 等 ~45 扩展名 | 围栏代码块（语言标签映射） |
| 图片 | png/jpg/jpeg/gif/bmp/webp/tiff/svg | 注入视觉模型时描述/OCR；否则元数据降级 |
| 音频 | wav/mp3/ogg/opus/flac/aac/m4a/wma | `asr` 转录或元数据降级 |
| 视频 | mp4/avi/mkv/mov/wmv/flv/webm/m4v | ffmpeg 抽轨 → 音频链 |
| 字幕 | srt/vtt/ass/ssa | 原生解析（时间码列表） |

完整清单以 `alch formats` 输出为准。

## 输出与资产

```bash
# dir 模式（默认）：图片写输出同目录 images/，重名自动加序号
alch convert slides.pptx -o slides.md          # → images/001.png …

# inline 模式：base64 data URL 内联，单文件交付
alch convert slides.pptx -o slides.md --assets inline
```

管道友好：不指定 `-o` 时 markdown 直接写 stdout。

```bash
alch convert paper.pdf | head -50
```

## 引擎选择与诊断

同一扩展名有多个引擎时按优先级降序尝试，前一个失败自动落到下一个。`--engine` 把偏好引擎提到队首（名称前缀匹配即可，如 `--engine any`；未命中静默回落，不报错）。

```bash
# 每个已注册引擎一行：可用状态 + 原因
alch doctor
# anydoc          AVAILABLE   doc,docm,pps,pot,xlsm,xlsb,odp,rtf,pdf,…
# pdf             AVAILABLE   pdf

# 全部支持扩展名
alch formats
```

注意：**未编译进特性的引擎、未配置端点的 MinerU 不会出现在列表中**（根本不注册）；媒体引擎的依赖缺失（无视觉提供方、无转录模型）在**转换时**降级产出元数据 markdown，doctor 仍显示 AVAILABLE。`--no-remote` 整体排除云端引擎（离线/合规场景）；`--format <ext>` 在扩展名不可信时强制指定格式（如把 `.md` 后缀的 PDF 喂对引擎）。

## PDF 高保真：MinerU 配置

设置端点后 MinerU 引擎才会注册；它排在本地链（anydoc → pdf-extract）之后，仅在本地链失败后作为高保真兜底（产出 `api` 级质量）：

```bash
export ALCHEMIZE_MINERU_URL="http://localhost:8888"   # 本地/网关 MinerU 服务
alch convert paper.pdf
```

云端 Precision API 需要令牌时用 `MINERU_API_TOKEN`（见[安全文档](SECURITY.md)）。超时默认 300 秒。

## 音视频转录（feature `asr`）

本地 ONNX 转录（Qwen3-ASR + silero VAD）需要模型 bundle 与 onnxruntime 动态库：

1. 模型目录布局：`qwen3-conv_frontend.onnx`、`qwen3-encoder.int8.onnx`、`qwen3-decoder.int8.onnx`、`silero_vad.onnx`、`tokenizer/`。
2. 用 `ALCHEMIZE_TRANSCRIBE_MODELS` 指向该目录；不设置则探测 `~/.cache/alchemize/models/transcribe/`。
3. `ORT_DYLIB_PATH` 指向 onnxruntime 动态库。

```bash
export ALCHEMIZE_TRANSCRIBE_MODELS=/path/to/transcribe-bundle
export ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib
alch convert meeting.mp4        # ffmpeg 抽轨 → VAD 分段转录
```

要点：

- bundle 缺失时引擎如实报 Unavailable 并附获取指引——**不会自动联网下载**。
- 输出是 VAD 段级文本 + 时间戳；VAD 是语音活动检测，**不是说话人识别**，输出不含说话人归属。
- Whisper 兼容云端后端仅在显式注入配置时激活（库 API `with_transcribe_provider`）。

## 离线与隐私

- 默认特性构建：不连任何外部服务。
- `--no-remote`：运行期排除云端引擎。
- ASR 完全本地推理；图片视觉模型需自行注入 provider（无内置云兜底）。

## 故障排查

| 现象 | 处置 |
| --- | --- |
| 退出码 2 `unsupported format: <ext>` | `alch formats` 确认扩展名；或用 `--format` 显式指定；需要的引擎可能未编译进特性 |
| 退出码 3 全链失败 | stderr 逐行列出各引擎失败原因，按原因处理（端点不可达/文件损坏/依赖缺失） |
| 产出质量只有 `local_fallback` | 属预期降级（如无 `anydoc` 时的 PDF 文本链）；要更高保真装 `anydoc`/配 MinerU |
| 图片只有元数据 | 未注入 VisionProvider；无内置兜底是设计行为 |
| ASR 报 Unavailable | 转换时报 `AudioUnavailable`（doctor 不显示 provider 级缺失）；核对 bundle 布局与 `ORT_DYLIB_PATH` |
| `zip` 转换报不支持嵌套 | zip 递归只支持一层容器内的普通文件；嵌套 zip 属显式不支持 |
