# ❓ Alchemize FAQ

常见问题解答。所有答案基于当前源码的真实行为，不含计划中或臆测的内容。

## 📋 目录

- [转换行为](#转换行为)
- [引擎与 fallback 链](#引擎与-fallback-链)
- [音视频与转录](#音视频与转录)
- [格式细节](#格式细节)

## 转换行为

### Q：转换降级了为什么退出码还是 0？

退出码表达「是否产出了 Markdown」：`0` 成功（含降级）、`2` 不支持格式、`3` 全链失败。降级属于「产出成功但保真度较低」——是否可接受由 stderr 摘要行的质量标签（`api`/`local_full`/`local_fallback`）判断，工具不替你做取舍，也不静默失败。

### Q：怎么知道这次转换用了哪个引擎、质量如何？

两个位置：stderr 摘要行 `converted X -> Y [quality] (engine: N)`（写文件时输出）；库使用者读结果 `metadata.converter`（实际引擎名）与 `quality` 字段。`quality` 由实际产出的引擎如实标注，与它在链中的位置无关。

### Q：为什么 `-o` 指定的输出旁边多了 `images/` 目录？

默认资产模式 `--assets dir`：文档内嵌图片按文档序抽取后写到输出同目录的 `images/`（重名自动加序号前缀），markdown 里用相对路径引用。要单文件交付用 `--assets inline`（base64 data URL 内联）。

## 引擎与 fallback 链

### Q：PDF 的引擎链是怎么排的？

以「本地优先、云端兜底」为原则，按注册表优先级降序：

1. `anydoc`（priority 15，feature `anydoc`，产出 `local_full`）
2. `pdf`——pdf-extract 本地文本提取 + TOC 剔除/结构启发（priority 10，产出 `local_fallback`）
3. `mineru`（priority 5，feature `mineru` 且设置了 `ALCHEMIZE_MINERU_URL`，产出 `api`）——仅在本地链全部失败后尝试

要强制某个引擎用 `--engine <name>`；要完全离线用 `--no-remote`。

### Q：`--engine` 写错名字会怎样？

静默回落默认优先级路由，不报错——过期的偏好配置不应阻塞转换。名称匹配支持前缀且大小写不敏感（`--engine any` 命中 `anydoc`）。

### Q：zip 递归转换的边界在哪？

zip 在门面层处理，与主注册表共享引擎集：压缩包内的普通文件按扩展名走各自引擎。**嵌套 zip 明确不支持**（报错而非递归）——zip 自身不注册进注册表正是为了避免无界自递归。

### Q：`doctor` 显示 UNAVAILABLE 的引擎还会被尝试吗？

不会。`Unavailable` 引擎在候选链中直接跳过（其原因此时记入 attempts），不消耗转换尝试次数。全链失败时 stderr 的明细里能看到它为何不可用。

## 音视频与转录

### Q：ASR 模型缺失时为什么报错而不是自动下载？

设计原则：不静默、不自动联网。模型 bundle 缺失时引擎如实报 `Unavailable` 并附获取指引——自动从网上拉模型涉及完整性与供应链问题，交给用户显式决策。布局要求见[用户指南](USER_GUIDE.md#音视频转录feature-asr)。

### Q：为什么转录结果没有说话人归属？

分段来自 silero VAD（语音活动检测），VAD 只回答「这段是不是语音」，不携带说话人身份。宁缺毋假：输出不虚构 SpeakerN 标签，且每份转录携带免责声明常量。这不是功能缺失被隐藏，是能力边界的显性声明。

### Q：视频直接转录吗？

不。视频先由 ffmpeg 抽出音轨，再走音频转录链；ffmpeg 不在 PATH 时视频引擎报 `VideoUnavailable`。帧去重与关键帧策略用于视觉描述路径，与转录互不影响。

## 格式细节

### Q：txt/csv 是乱码时怎么办？支持哪些编码？

解码顺序（`src/infrastructure/document/shared.rs` 的 `detect_encoding`）：先按 UTF-8 直读；失败后按 **GBK → Big5 → Shift-JIS → EUC-JP → ISO-2022-JP → EUC-KR → windows-1252** 探测（CJK 优先，因为 windows-1252 几乎能解码任何字节序列、放前面会遮蔽 CJK 内容）；全部失败落 lossy UTF-8（坏字节变 U+FFFD，不报错）。csv 另有分隔符自探测（首行 tab 多于逗号按 TSV 处理）。

### Q：`.md` 文件交给谁处理？

`md` 归入纯文本族（`txt/text/log/md`）由 `text` 引擎处理；代码/配置扩展名（py/rs/json/toml/…）输出带语言标签的围栏代码块。

### Q：旧版二进制 Office（`.doc`/`.ppt`）为什么转不了？

原生 XML 解析器只认 OOXML（docx/pptx/xlsx）。旧版格式由 `anydoc` 后端支持——需以 `--features anydoc`（或 `full`）编译。未编译该特性时这些扩展名不在 `alch formats` 清单中。

### Q：LaTeX 必须装 pandoc 吗？

不必。`latex-source` 引擎优先探测 pandoc（PATH 上有则用其转换）；缺失时退回围栏兜底（源码按 latex 代码块输出，质量标注 `local_fallback`），转换仍然成功——该引擎的 doctor 状态恒为 AVAILABLE。
