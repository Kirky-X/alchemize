# Alchemize user-facing messages (Simplified Chinese).
# Key set must stay identical to locales/en/messages.ftl (guard test:
# en_and_zh_key_sets_match). English words kept inline: product names
# (MinerU, ffmpeg), flags (--features), and codeset terms.

# ── domain/error.rs — AlchemizeError Display ────────────────────────────
unsupported-format-for-input = 输入格式不支持：{ $what }
container-nesting-exceeds-the-depth-limit-of = 容器嵌套超过深度上限 { $limit }
no-converter-accepted-the-input = 没有转换器接受该输入
all-backends-failed-for = { $ext } 的全部后端均失败：{ $attempts }
failed-to-read-input-file = 读取输入文件 '{ $path }' 失败：{ $source }
pdf-extraction-failed = PDF 文本抽取失败：{ $what }
zip-archive-error = ZIP 归档错误：{ $what }
xml-parsing-error = XML 解析错误：{ $what }
video-processing-unavailable = 视频处理不可用：{ $what }
audio-transcription-unavailable = 音频转写不可用：{ $what }
vision-provider-unavailable = 视觉服务不可用：{ $what }
mineru-service-unavailable = MinerU 服务不可用：{ $what }
mineru-conversion-error = MinerU 转换错误：{ $what }
security-violation = 安全违规（{ $kind }）：{ $message }
response-body-too-large = 响应体过大：{ $actual } 字节，超过上限 { $limit } 字节
synchronous-conversion-not-available = 同步转换不可用：{ $what }
engine-has-no-synchronous-path = 引擎 '{ $engine }' 没有同步路径

# ── domain/error.rs — SecurityViolationType Display ─────────────────────
invalid-scheme = 协议不合法
loopback-address = 环回地址
private-address = 私有地址
link-local-address = 链路本地地址
hostname-not-allowed = 主机名不在允许列表
missing-signature = 缺少签名

# ── forge_app.rs — diagnostics & endpoint output ────────────────────────
unsupported-format = 不支持的格式：{ $ext }
unsupported-format-requires-feature = 不支持的格式：{ $ext }：需要 { $names } feature{ $plural }；本构建未启用，请用 --features { $flags } 重新构建
all-backends-failed-header = { $ext } 的全部后端均失败：
warning-base-url-extract-main-require-html = 警告：--base-url/--extract-main 需要 --features html 构建；选项已忽略
input-file-not-found = 输入文件不存在：{ $path }
unknown-asset-mode = 未知的 asset 模式 '{ $what }'（dir|inline）
failed-to-read-input = 读取输入失败：{ $source }
asset-filename-rejected-by-write-boundary = 资产文件名被写入边界拒绝：{ $reason }
write-failed = 写入失败：{ $source }
asset-write-failed = 资产写入失败：{ $source }
converted-to = 已转换 { $input } -> { $output } [{ $quality }]（引擎：{ $engine }）

# ── main.rs / alch_mcp.rs — runner messages ─────────────────────────────
alch-logging-init-failed = alch：日志初始化失败：{ $source }
alch-log-shutdown-failed = alch：日志关闭失败：{ $source }
alch-mcp-logging-init-failed = alch-mcp：日志初始化失败：{ $source }
alch-mcp-log-shutdown-failed = alch-mcp：日志关闭失败：{ $source }

# ── application/registry.rs — fallback-chain logging ────────────────────
converter-failed-trying-next = 转换器失败，尝试下一个
converter-failed-trying-next-sync = 转换器失败，尝试下一个（同步）
unavailable = 不可用：{ $reason }

# ── infrastructure/document/html_deep.rs ────────────────────────────────
html-deep-base-url-not-valid = html-deep：base_url 不是合法 URL；链接保持原样
html-deep-main-content-extraction-failed = html-deep：主内容抽取失败，回退为整页转换
html-deep-extraction-empty-fallback-full-page = html-deep：抽取产物为空，回退为整页转换
html-deep-primary-empty-visible-text-fallback-htmd = html-deep：主路径产物为空但页面有可见文本，回退为 htmd 兜底

# ── infrastructure/media ────────────────────────────────────────────────
srt-parse-error = SRT 解析错误：{ $source }
vtt-parse-error = VTT 解析错误：{ $source }
ass-parse-error = ASS 解析错误：{ $source }
ffmpeg-not-found-video-degraded = 未找到 ffmpeg — 视频处理降级为仅元数据
audio-extraction-failed = '{ $filename }' 音频抽取失败：{ $source }

# ── infrastructure/remote/mineru ────────────────────────────────────────
mineru-task-submitted = MinerU 任务已提交
mineru-task-completed = MinerU 任务已完成
unknown-task-status = 未知任务状态

# ── catalog.rs self-test (key-set parity guard) ─────────────────────────
test-args = 值 { $n }
