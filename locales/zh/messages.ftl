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

# ── main.rs / mcp_cmd.rs — runner messages ──────────────────────────────
alch-logging-init-failed = alch：日志初始化失败：{ $source }
alch-log-shutdown-failed = alch：日志关闭失败：{ $source }
mcp-stdio-server-failed = alch mcp：stdio 服务失败：{ $source }

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

# ── application/ports.rs — transcript disclaimer ────────────────────────
transcript-disclaimer = 分段由 VAD（语音活动检测）产生，非说话人识别；不提供说话人归属。

# ── infrastructure/remote/mineru ────────────────────────────────────────
mineru-task-submitted = MinerU 任务已提交
mineru-task-completed = MinerU 任务已完成
unknown-task-status = 未知任务状态

# ── i18n 完备性修复：生产代码硬编码文案 ────────────────────────────────
input-size-exceeds-hard-limit = 输入大小 { $len } 字节超过硬输入上限 { $limit } 字节（1 GiB）
input-size-exceeds-max-file-size = 输入大小 size { $len } bytes exceeds max_file_size { $limit } 字节（上限）
path-input-requires-async-port = 路径输入需要异步端口；同步端口仅处理内存中的 Input::Bytes
zip-cbz-requires-async-port = zip/cbz 需要嵌套异步分发；请使用异步端口
program-not-found-on-path = 未在 PATH 中找到 '{ $program }'
empty-asset-filename = 资产文件名为空
asset-filename-contains-nul = 资产文件名包含 NUL
asset-filename-contains-backslash = 资产文件名包含反斜杠
absolute-asset-filename = 绝对资产文件名：{ $filename }
colon-in-asset-filename = 资产文件名含冒号：{ $filename }
empty-path-component-in-filename = 路径含空组件：{ $filename }
path-traversal-component-in-filename = 路径含穿越组件：{ $filename }
pdf-cumulative-text-budget-exceeded = PDF cumulative text budget exceeded（共 { $limit } 字节，bomb protection；page-farm DoS 防护）
pdf-page-decompression-limit-exceeded = PDF page decompression limit exceeded（每页 { $limit } 字节，bomb protection；lopdf 内置限制）
failed-to-read-pdf-input = 读取 PDF 输入失败：{ $source }
pdf-extraction-task-failed = PDF 抽取任务失败：{ $source }
html-conversion-failed = HTML 转换失败：{ $source }
html-no-extractable-content = *HTML contains no extractable content.*（HTML 文档没有可抽取的内容。）
failed-to-read-html-input = 读取 HTML 输入失败：{ $source }
html-content-not-valid-utf8 = HTML 内容不是合法的 UTF-8：{ $source }
failed-to-read-eml-input = 读取 EML 输入失败：{ $source }
eml-parse-failed = EML 解析失败：不是合法的 RFC 822 邮件
pdfium-task-panicked = pdfium 任务崩溃：{ $source }
pdfium-bind-failed = pdfium 绑定失败：{ $source }
pdfium-open-failed = pdfium 打开失败：{ $source }
soffice-not-found-install = 未在 PATH 中找到 'soffice'（请安装 LibreOffice）
soffice-version-exited = soffice --version 退出码 { $status }：{ $stderr }
soffice-probe-failed = soffice 探测失败：{ $source }
failed-to-read-ext-input = 读取 { $ext } 输入失败：{ $source }
tempdir-failed = 创建临时目录失败：{ $source }
staging-failed = 暂存失败：{ $source }
soffice-profile-failed = soffice 配置目录失败：{ $source }
soffice-conversion-failed = soffice 转换失败：{ $source }
converted-output-unreadable = 转换产物不可读：{ $source }
unsupported-intermediate-format = 不支持的中间格式 '{ $other }'
failed-to-read-ppt-input = 读取 ppt 输入失败：{ $source }
soffice-not-found-doc-rtf = 未在 PATH 中找到 'soffice'（旧版 doc/rtf 需要 LibreOffice）
soffice-not-found-legacy-ppt = 未在 PATH 中找到 'soffice'（旧版 ppt 需要 LibreOffice）
failed-to-open-docx = 打开 DOCX 失败：{ $source }
failed-to-open-pptx = 打开 PPTX 失败：{ $source }
part-not-found = 未找到部件 { $name }：{ $source }
xml-parse-error-at-position = { $position } 处 XML 解析错误：{ $source }
failed-to-read-odt-input = 读取 ODT 输入失败：{ $source }
failed-to-read-pptx-input = 读取 PPTX 输入失败：{ $source }
failed-to-read-docx-input = 读取 DOCX 输入失败：{ $source }
docx-deep-task-panicked = docx-deep 任务崩溃：{ $source }
pptx-deep-task-panicked = pptx-deep 任务崩溃：{ $source }
odt-deep-task-panicked = odt-deep 任务崩溃：{ $source }
odp-deep-task-panicked = odp-deep 任务崩溃：{ $source }
failed-to-read-odp-input = 读取 ODP 输入失败：{ $source }
failed-to-open-odf-package = 打开 ODF 包失败：{ $source }
content-xml-parse-error = content.xml parse error（解析错误）：{ $source }
document-xml-parse-error = document.xml parse error（解析错误）：{ $source }
slide-parse-error = 幻灯片解析错误：{ $source }
odt-table-repeat-budget = { $kind } 表格重复展开超过 { $limit } 字节输出预算（output budget）
failed-to-read-notebook-input = 读取 notebook 输入失败：{ $source }
notebook-not-utf8 = notebook 内容不是合法的 UTF-8：{ $source }
failed-to-parse-notebook-json = 解析 notebook JSON 失败：{ $source }
failed-to-read-epub-input = 读取 EPUB 输入失败：{ $source }
chapter-html-conversion-failed = 章节 HTML 转换失败：{ $source }
failed-to-read-pandoc-input = 读取 .{ $ext } 输入失败：{ $source }
pandoc-failed = pandoc 失败：{ $source }
failed-to-read-text-input = 读取文本输入失败：{ $source }
failed-to-read-csv-input = 读取 CSV 输入失败：{ $source }
csv-parse-error = CSV 解析错误：{ $source }
failed-to-read-csv-headers = 读取 CSV 表头失败：{ $source }
failed-to-read-latex-input = 读取 LaTeX 输入失败：{ $source }
chm-extraction-task-failed = CHM 抽取任务失败：{ $source }
failed-to-create-temp-for-chm = 为 CHM 创建临时文件失败：{ $source }
failed-to-spool-chm-input = 写入 CHM 输入临时文件失败：{ $source }
chm-open-failed = CHM 打开失败：{ $source }
chm-enumeration-failed = CHM 枚举失败：{ $source }
chm-topic-failed = CHM 主题 { $topic } 失败：{ $source }
failed-to-read-zip-input = 读取 ZIP 输入失败：{ $source }
failed-to-open-zip-archive = 打开 ZIP 归档失败：{ $source }
failed-to-read-zip-entry = 读取 ZIP 条目 { $index } 失败：{ $source }
failed-to-read-zip-entry-name = 读取 ZIP 条目名 { $index } 失败：{ $source }
failed-to-read-file-content = 读取文件内容失败：{ $source }
zip-cumulative-exceeded = cumulative extracted size exceeds { $limit } bytes（累计解压大小超限）— 中止（可能的解压炸弹）
part-read-failed = 读取部件 { $what } 失败：{ $source }
part-per-part-limit = 部件 { $what } 超过 { $limit } 字节单部件上限（per-part limit）
output-cumulative-limit = 转换产物超过 { $limit } 字节累计上限
word-document-xml-not-found = 未找到 word/document.xml：{ $source }
failed-to-read-tar-input = 读取 tar 输入失败：{ $source }
failed-to-read-targz-input = 读取 tar.gz 输入失败：{ $source }
failed-to-read-tar-archive = 读取 tar 归档失败：{ $source }
failed-to-read-tar-entry = 读取 tar 条目失败：{ $source }
failed-to-read-tar-entry-name = 读取 tar 条目名失败：{ $source }
failed-to-read-tar-entry-size = 读取 tar 条目大小失败：{ $source }
failed-to-read-tar-entry-named = 读取 tar 条目 '{ $name }' 失败：{ $source }
zst-decoder-start-failed = zst：启动解码器失败：{ $source }
failed-to-read-format-input = 读取 { $format } 输入失败：{ $source }
failed-to-read-7z-input = 读取 7z 输入失败：{ $source }
failed-to-open-7z-archive = 打开 7z 归档失败：{ $source }
sevenz-per-entry-limit = 条目 '{ $name }' 为 { $size } 字节，超过 { $limit } 字节单条目上限
failed-to-extract-7z-entries = 解压 7z 条目失败：{ $source }
chain-decompressed-size-exceeded = 容器链解压大小超过 { $limit } 字节安全上限（chain decompressed size / safety limit）
depth-scope-not-entered = 未进入容器深度作用域：容器转换器必须在 with_depth_scope 内运行
container-size-exceeded = 解压容器大小 { $total } 字节超过 { $limit } 字节安全上限（safety limit）
entry-size-exceeded = 条目 '{ $name }' 为 { $size } 字节，超过 { $limit } 字节单条目上限
container-read-failed = { $context }：{ $source }
container-decompressed-size-exceeded = { $context }：解压大小超过 { $limit } 字节安全上限（safety limit）
container-registry-slot-not-wired = 容器注册表槽位未接线
failed-to-stage-sqlite-temp = 暂存 SQLite 临时文件失败：{ $source }
failed-to-read-sqlite-input = failed to read sqlite input（读取失败）：{ $source }
failed-to-open-sqlite-readonly = 以只读方式打开 SQLite 数据库失败：{ $source }
failed-to-list-sqlite-tables = 列出 SQLite 表失败：{ $source }
failed-to-query-sqlite-tables = 查询 SQLite 表失败：{ $source }
failed-to-read-sqlite-tables = 读取 SQLite 表失败：{ $source }
failed-to-inspect-sqlite-table = 检查表 '{ $table }' 失败：{ $source }
failed-to-sample-sqlite-table = 采样表 '{ $table }' 失败：{ $source }
failed-to-read-parquet-input = failed to read parquet input（读取失败）：{ $source }
failed-to-open-parquet-file = 打开 parquet 文件失败：{ $source }
failed-to-read-parquet-rows = 读取 parquet 行失败：{ $source }
failed-to-read-parquet-row-n = 读取 parquet 第 { $n } 行失败：{ $source }
failed-to-read-bib-input = failed to read bib input（读取失败）：{ $source }
bib-not-utf8 = bib 文件不是合法的 UTF-8 文本
failed-to-parse-bibtex = 解析 BibTeX 失败：{ $source }
failed-to-read-subtitle-input = 读取字幕输入失败：{ $source }
failed-to-read-image-input = 读取图片输入失败：{ $source }
ocr-failed = OCR 失败：{ $source }
image-analysis-failed = 分析失败：{ $source }
failed-to-read-video-input = 读取视频输入失败：{ $source }
transcription-failed = 转写失败：{ $source }
mime-error = MIME 错误：{ $source }
failed-to-read-audio-input = 读取 audio 输入失败：{ $source }
failed-to-read-heic-input = 读取 HEIC 输入失败：{ $source }
ort-dylib-not-set = 未设置 ORT_DYLIB_PATH；vecboost 动态加载 onnxruntime
vecboost-bundle-missing-vad = { $path } 处的模型包缺少 silero_vad.onnx；模型包不完整
vecboost-tempfile-failed = 创建临时文件失败：{ $source }
vecboost-wav-write-failed = 写入 wav 失败：{ $source }
vecboost-task-panicked = vecboost 任务崩溃：{ $source }
vecboost-transcription-failed = vecboost 转写失败：{ $source }
converter-module-require-failed = ConverterModule：require alchemize 失败：{ $source }
failed-to-build-http-client = 构建 HTTP 客户端失败：{ $source }
cannot-reach-mineru = 无法连接 MinerU（cannot reach MinerU at { $base }）：{ $source }
failed-to-parse-mineru-response = 解析 MinerU 响应失败（failed to parse MinerU response）：{ $source }
failed-to-parse-mineru-poll-response = 解析 MinerU 轮询响应失败（failed to parse MinerU poll response）：{ $source }
mineru-poll-returned = MinerU 轮询返回 { $status }：{ $body }
mineru-task-timed-out = MinerU 任务 { $task } 在 { $secs } 秒后超时
mineru-task-failed = MinerU 任务 { $task } 失败：{ $error }
mineru-unknown-error = 未知错误
polling-mineru-task = 轮询 MinerU 任务 { $task }：{ $source }
failed-to-read-mineru-input = 读取输入失败（failed to read input）：{ $source }
mineru-api-returned = MinerU API 返回 { $status }：{ $body }
mineru-agent-api-returned = MinerU Agent API 返回 { $status }：{ $body }
mineru-token-not-set = 未设置 MINERU_API_TOKEN 环境变量（Precision API 必需）
mineru-token-empty = MinerU API token 不能为空
mineru-empty-task-id = MinerU 返回了空的 task_id
mineru-task-id-disallowed = MinerU 返回的 task_id 含非法字符：{ $id }
url-points-to-loopback = URL 指向 { $label }环回地址 '{ $host }'
url-points-to-private = URL 指向 { $label }私有地址 '{ $host }'
url-points-to-link-local = URL 指向 { $label }链路本地地址 '{ $host }'
url-points-to-broadcast = URL 指向 { $label }广播地址 '{ $host }'
invalid-file-url = URL 无效 '{ $raw }'：{ $source }
url-must-use-https = URL 必须使用 HTTPS 协议，实际为 '{ $scheme }'
url-has-no-host = URL '{ $raw }' 没有主机
url-ipv6-ula = URL 指向 IPv6 ULA 地址 '{ $host }'
url-ipv6-link-local = URL 指向 IPv6 链路本地地址 '{ $host }'
url-ipv6-multicast = URL 指向 IPv6 组播地址 '{ $host }'
hostname-not-in-allowlist = 主机名 '{ $host }' 不在配置的允许列表中
invalid-mineru-url = MinerU URL 无效 '{ $url }'：{ $source }
mineru-url-scheme-invalid = MinerU URL 必须使用 http 或 https 协议，实际为 '{ $scheme }'
nested-container-depth-limit = nested container exceeds the depth limit（嵌套容器超过深度上限）
unsupported-inner-format = unsupported inner format（不支持的内部格式）
attachment-markdown = > Attachment：`{ $name }`（{ $size } 字节）— { $reason }；内容已作为附件保留。
zip-no-convertible-files = *ZIP archive contains no convertible files.*（ZIP 归档中没有可转换的文件。）
sqlite-sample-rows-none = Sample rows: *(none)*（示例行：无）
sqlite-sample-rows-header = Sample rows (first { $count })（示例行前 { $count } 条）

# ── i18n phase 4: remaining converter details ──────────────────────────
pdfium-library-unavailable = pdfium 库不可用
failed-to-read-msg-input = failed to read MSG input（读取 MSG 输入失败）：{ $source }
msg-extraction-task-failed = MSG extraction task failed（MSG 抽取任务失败）：{ $source }
msg-compound-file-open-failed = MSG compound file open failed（MSG 复合文件打开失败）：{ $source }
msg-properties-read-failed = MSG properties read failed（MSG 属性读取失败）：{ $source }
failed-to-open-epub = failed to open EPUB（打开 EPUB 失败）：{ $source }
whisper-api-request-failed = whisper api request failed（请求失败）：{ $source }
whisper-api-status = whisper api status（状态异常）：{ $source }
whisper-api-decode = whisper api decode（解码失败）：{ $source }
failed-reading-response-stream = failed reading response stream（读取响应流失败）：{ $source }
response-body-not-utf8 = response body is not valid UTF-8（响应体非 UTF-8）：{ $source }
failed-to-parse-json-response = failed to parse JSON response（解析 JSON 响应失败）：{ $source }
failed-to-fetch-result = failed to fetch result（获取结果失败）：{ $source }
failed-to-read-result = failed to read result（读取结果失败）：{ $source }

# ── domain/provenance.rs — EngineStep Display outcome labels ────────────
engine-step-skipped = 已跳过
engine-step-failed = 失败
engine-step-used = 已使用

# ── catalog.rs self-test (key-set parity guard) ─────────────────────────
test-args = 值 { $n }
