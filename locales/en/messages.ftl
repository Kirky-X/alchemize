# Alchemize user-facing messages (English).
# Keys are the English source text itself: the en bundle (and the missing-key
# fallback) renders byte-identical to the pre-i18n build. Fluent placeables
# { $name } interpolate String args.

# ── domain/error.rs — AlchemizeError Display ────────────────────────────
unsupported-format-for-input = unsupported format for input: { $what }
container-nesting-exceeds-the-depth-limit-of = container nesting exceeds the depth limit of { $limit }
no-converter-accepted-the-input = no converter accepted the input
all-backends-failed-for = all backends failed for { $ext }: { $attempts }
failed-to-read-input-file = failed to read input file '{ $path }': { $source }
pdf-extraction-failed = PDF extraction failed: { $what }
zip-archive-error = ZIP archive error: { $what }
xml-parsing-error = XML parsing error: { $what }
video-processing-unavailable = video processing unavailable: { $what }
audio-transcription-unavailable = audio transcription unavailable: { $what }
vision-provider-unavailable = vision provider unavailable: { $what }
mineru-service-unavailable = MinerU service unavailable: { $what }
mineru-conversion-error = MinerU conversion error: { $what }
security-violation = security violation ({ $kind }): { $message }
response-body-too-large = response body too large: { $actual } bytes exceeds limit of { $limit } bytes
synchronous-conversion-not-available = synchronous conversion not available: { $what }
engine-has-no-synchronous-path = engine '{ $engine }' has no synchronous path

# ── domain/error.rs — SecurityViolationType Display ─────────────────────
invalid-scheme = invalid scheme
loopback-address = loopback address
private-address = private address
link-local-address = link-local address
hostname-not-allowed = hostname not allowed
missing-signature = missing signature

# ── forge_app.rs — diagnostics & endpoint output ────────────────────────
unsupported-format = unsupported format: { $ext }
unsupported-format-requires-feature = unsupported format: { $ext }: requires the { $names } feature{ $plural }; this build has { $them } disabled, rebuild with --features { $flags }
all-backends-failed-header = all backends failed for { $ext }:
warning-base-url-extract-main-require-html = warning: --base-url/--extract-main require a build with --features html; options ignored
input-file-not-found = input file not found: { $path }
unknown-asset-mode = unknown asset mode '{ $what }' (dir|inline)
failed-to-read-input = failed to read input: { $source }
asset-filename-rejected-by-write-boundary = asset filename rejected by write boundary: { $reason }
write-failed = write failed: { $source }
asset-write-failed = asset write failed: { $source }
converted-to = converted { $input } -> { $output } [{ $quality }] (engine: { $engine })

# ── main.rs / mcp_cmd.rs — runner messages ──────────────────────────────
alch-logging-init-failed = alch: logging init failed: { $source }
alch-log-shutdown-failed = alch: log shutdown failed: { $source }
mcp-stdio-server-failed = alch mcp: stdio server failed: { $source }

# ── application/registry.rs — fallback-chain logging ────────────────────
converter-failed-trying-next = converter failed, trying next
converter-failed-trying-next-sync = converter failed, trying next (sync)
unavailable = unavailable: { $reason }

# ── infrastructure/document/html_deep.rs ────────────────────────────────
html-deep-base-url-not-valid = html-deep: base_url is not a valid URL; links left unchanged
html-deep-main-content-extraction-failed = html-deep: main-content extraction failed, falling back to full page
html-deep-extraction-empty-fallback-full-page = html-deep: extraction produced empty output, falling back to full page
html-deep-primary-empty-visible-text-fallback-htmd = html-deep: primary produced empty output with visible text, falling back to htmd

# ── infrastructure/media ────────────────────────────────────────────────
srt-parse-error = SRT parse error: { $source }
vtt-parse-error = VTT parse error: { $source }
ass-parse-error = ASS parse error: { $source }
ffmpeg-not-found-video-degraded = ffmpeg not found — video processing degraded to metadata only
audio-extraction-failed = audio extraction failed for '{ $filename }': { $source }

# ── application/ports.rs — transcript disclaimer ────────────────────────
transcript-disclaimer = Segments are produced by VAD (voice activity detection), not speaker identification; no speaker attribution is provided.

# ── infrastructure/remote/mineru ────────────────────────────────────────
mineru-task-submitted = MinerU task submitted
mineru-task-completed = MinerU task completed
unknown-task-status = unknown task status

# ── i18n completeness fix: hardcoded production messages ────────────────
input-size-exceeds-hard-limit = input size { $len } bytes exceeds the hard input limit of { $limit } bytes (1 GiB)
input-size-exceeds-max-file-size = input size { $len } bytes exceeds max_file_size { $limit } bytes
path-input-requires-async-port = path input requires the async port; the sync port serves in-memory Input::Bytes only
zip-cbz-requires-async-port = zip/cbz requires nested async dispatch; use the async port
program-not-found-on-path = '{ $program }' not found on PATH
empty-asset-filename = empty asset filename
asset-filename-contains-nul = asset filename contains NUL
asset-filename-contains-backslash = asset filename contains backslash
absolute-asset-filename = absolute asset filename: { $filename }
colon-in-asset-filename = colon in asset filename: { $filename }
empty-path-component-in-filename = empty path component in { $filename }
path-traversal-component-in-filename = path traversal component in { $filename }
pdf-cumulative-text-budget-exceeded = PDF cumulative text budget exceeded ({ $limit } bytes total, bomb protection; page-farm DoS guard)
pdf-page-decompression-limit-exceeded = PDF page decompression limit exceeded ({ $limit } bytes/page, bomb protection; lopdf built-in limits)
failed-to-read-pdf-input = failed to read PDF input: { $source }
pdf-extraction-task-failed = pdf extraction task failed: { $source }
html-conversion-failed = HTML conversion failed: { $source }
html-no-extractable-content = *HTML document contains no extractable content.*
failed-to-read-html-input = failed to read HTML input: { $source }
html-content-not-valid-utf8 = HTML content is not valid UTF-8: { $source }
failed-to-read-eml-input = failed to read EML input: { $source }
eml-parse-failed = EML parse failed: not a valid RFC 822 message
pdfium-task-panicked = pdfium task panicked: { $source }
pdfium-bind-failed = pdfium bind failed: { $source }
pdfium-open-failed = pdfium open failed: { $source }
soffice-not-found-install = 'soffice' not found on PATH (install LibreOffice)
soffice-version-exited = soffice --version exited { $status }: { $stderr }
soffice-probe-failed = soffice probe failed: { $source }
failed-to-read-ext-input = failed to read { $ext } input: { $source }
tempdir-failed = tempdir failed: { $source }
staging-failed = staging failed: { $source }
soffice-profile-failed = soffice profile failed: { $source }
soffice-conversion-failed = soffice conversion failed: { $source }
converted-output-unreadable = converted output unreadable: { $source }
unsupported-intermediate-format = unsupported intermediate format '{ $other }'
failed-to-read-ppt-input = failed to read ppt input: { $source }
soffice-not-found-doc-rtf = 'soffice' not found on PATH (legacy doc/rtf need LibreOffice)
soffice-not-found-legacy-ppt = 'soffice' not found on PATH (legacy ppt needs LibreOffice)
failed-to-open-docx = failed to open DOCX: { $source }
failed-to-open-pptx = failed to open PPTX: { $source }
part-not-found = part { $name } not found: { $source }
xml-parse-error-at-position = XML parse error at position { $position }: { $source }
failed-to-read-odt-input = failed to read ODT input: { $source }
failed-to-read-pptx-input = failed-to-read PPTX input: { $source }
failed-to-read-docx-input = failed to read DOCX input: { $source }
docx-deep-task-panicked = docx-deep task panicked: { $source }
pptx-deep-task-panicked = pptx-deep task panicked: { $source }
odt-deep-task-panicked = odt-deep task panicked: { $source }
odp-deep-task-panicked = odp-deep task panicked: { $source }
failed-to-read-odp-input = failed to read ODP input: { $source }
failed-to-open-odf-package = failed to open ODF package: { $source }
content-xml-parse-error = content.xml parse error: { $source }
document-xml-parse-error = document.xml parse error: { $source }
slide-parse-error = slide parse error: { $source }
odt-table-repeat-budget = { $kind } table repeat expansion exceeds the { $limit }-byte output budget
failed-to-read-notebook-input = failed to read notebook input: { $source }
notebook-not-utf8 = notebook content is not valid UTF-8: { $source }
failed-to-parse-notebook-json = failed to parse notebook JSON: { $source }
failed-to-read-epub-input = failed to read EPUB input: { $source }
chapter-html-conversion-failed = chapter HTML conversion failed: { $source }
failed-to-read-pandoc-input = failed to read .{ $ext } input: { $source }
pandoc-failed = pandoc failed: { $source }
failed-to-read-text-input = failed to read text input: { $source }
failed-to-read-csv-input = failed to read CSV input: { $source }
csv-parse-error = CSV parse error: { $source }
failed-to-read-csv-headers = failed to read CSV headers: { $source }
failed-to-read-latex-input = failed to read LaTeX input: { $source }
chm-extraction-task-failed = CHM extraction task failed: { $source }
failed-to-create-temp-for-chm = failed to create temp file for CHM: { $source }
failed-to-spool-chm-input = failed to spool CHM input: { $source }
chm-open-failed = CHM open failed: { $source }
chm-enumeration-failed = CHM enumeration failed: { $source }
chm-topic-failed = CHM topic { $topic } failed: { $source }
failed-to-read-zip-input = failed to read ZIP input: { $source }
failed-to-open-zip-archive = failed to open ZIP archive: { $source }
failed-to-read-zip-entry = failed to read ZIP entry { $index }: { $source }
failed-to-read-zip-entry-name = failed to read ZIP entry name { $index }: { $source }
failed-to-read-file-content = failed to read file content: { $source }
zip-cumulative-exceeded = cumulative extracted size exceeds { $limit } bytes — aborting (possible decompression bomb)
part-read-failed = part { $what } read failed: { $source }
part-per-part-limit = part { $what } exceeds the { $limit }-byte per-part limit
output-cumulative-limit = converted output exceeds the { $limit }-byte cumulative limit
word-document-xml-not-found = word/document.xml not found: { $source }
failed-to-read-tar-input = failed to read tar input: { $source }
failed-to-read-targz-input = failed to read tar.gz input: { $source }
failed-to-read-tar-archive = failed to read tar archive: { $source }
failed-to-read-tar-entry = failed to read tar entry: { $source }
failed-to-read-tar-entry-name = failed to read tar entry name: { $source }
failed-to-read-tar-entry-size = failed to read tar entry size: { $source }
failed-to-read-tar-entry-named = failed to read tar entry '{ $name }': { $source }
zst-decoder-start-failed = zst: failed to start decoder: { $source }
failed-to-read-format-input = failed to read { $format } input: { $source }
failed-to-read-7z-input = failed to read 7z input: { $source }
failed-to-open-7z-archive = failed to open 7z archive: { $source }
sevenz-per-entry-limit = entry '{ $name }' is { $size } bytes, exceeds the { $limit }-byte per-entry limit
failed-to-extract-7z-entries = failed to extract 7z entries: { $source }
chain-decompressed-size-exceeded = container chain decompressed size exceeds the { $limit }-byte safety limit
depth-scope-not-entered = container depth scope not entered: container converters must run inside with_depth_scope
container-size-exceeded = decompressed container size { $total } bytes exceeds the { $limit }-byte safety limit
entry-size-exceeded = entry '{ $name }' is { $size } bytes, exceeds the { $limit }-byte per-entry limit
container-read-failed = { $context }: { $source }
container-decompressed-size-exceeded = { $context }: decompressed size exceeds the { $limit }-byte safety limit
container-registry-slot-not-wired = container registry slot is not wired
failed-to-stage-sqlite-temp = failed to stage SQLite temp file: { $source }
failed-to-read-sqlite-input = failed to read sqlite input: { $source }
failed-to-open-sqlite-readonly = failed to open SQLite database read-only: { $source }
failed-to-list-sqlite-tables = failed to list SQLite tables: { $source }
failed-to-query-sqlite-tables = failed to query SQLite tables: { $source }
failed-to-read-sqlite-tables = failed to read SQLite tables: { $source }
failed-to-inspect-sqlite-table = failed to inspect table '{ $table }': { $source }
failed-to-sample-sqlite-table = failed to sample table '{ $table }': { $source }
failed-to-read-parquet-input = failed to read parquet input: { $source }
failed-to-open-parquet-file = failed to open parquet file: { $source }
failed-to-read-parquet-rows = failed to read parquet rows: { $source }
failed-to-read-parquet-row-n = failed to read parquet row { $n }: { $source }
failed-to-read-bib-input = failed to read bib input: { $source }
bib-not-utf8 = bib file is not valid UTF-8 text
failed-to-parse-bibtex = failed to parse BibTeX: { $source }
failed-to-read-subtitle-input = failed to read subtitle input: { $source }
failed-to-read-image-input = failed to read image input: { $source }
ocr-failed = OCR failed: { $source }
image-analysis-failed = analysis failed: { $source }
failed-to-read-video-input = failed to read video input: { $source }
transcription-failed = transcription failed: { $source }
mime-error = mime error: { $source }
failed-to-read-audio-input = failed to read audio input: { $source }
failed-to-read-heic-input = failed to read HEIC input: { $source }
ort-dylib-not-set = ORT_DYLIB_PATH is not set; vecboost loads onnxruntime dynamically
vecboost-bundle-missing-vad = bundle at { $path } misses silero_vad.onnx; incomplete bundle
vecboost-tempfile-failed = tempfile failed: { $source }
vecboost-wav-write-failed = wav write failed: { $source }
vecboost-task-panicked = vecboost task panicked: { $source }
vecboost-transcription-failed = vecboost transcription failed: { $source }
converter-module-require-failed = ConverterModule: require alchemize: { $source }
failed-to-build-http-client = failed to build HTTP client: { $source }
cannot-reach-mineru = cannot reach MinerU at { $base }: { $source }
failed-to-parse-mineru-response = failed to parse MinerU response: { $source }
failed-to-parse-mineru-poll-response = failed to parse MinerU poll response: { $source }
mineru-poll-returned = MinerU poll returned { $status }: { $body }
mineru-task-timed-out = MinerU task { $task } timed out after { $secs }s
mineru-task-failed = MinerU task { $task } failed: { $error }
mineru-unknown-error = unknown error
polling-mineru-task = polling MinerU task { $task }: { $source }
failed-to-read-mineru-input = failed to read input: { $source }
mineru-api-returned = MinerU API returned { $status }: { $body }
mineru-agent-api-returned = MinerU Agent API returned { $status }: { $body }
mineru-token-not-set = MINERU_API_TOKEN environment variable not set. Required for Precision API.
mineru-token-empty = MinerU API token cannot be empty
mineru-empty-task-id = MinerU returned an empty task_id
mineru-task-id-disallowed = MinerU returned a task_id with disallowed characters: { $id }
url-points-to-loopback = URL points to { $label }loopback address '{ $host }'
url-points-to-private = URL points to { $label }private address '{ $host }'
url-points-to-link-local = URL points to { $label }link-local address '{ $host }'
url-points-to-broadcast = URL points to { $label }broadcast address '{ $host }'
invalid-file-url = invalid URL '{ $raw }': { $source }
url-must-use-https = URL must use HTTPS scheme, got '{ $scheme }'
url-has-no-host = URL '{ $raw }' has no host
url-ipv6-ula = URL points to IPv6 ULA address '{ $host }'
url-ipv6-link-local = URL points to IPv6 link-local address '{ $host }'
url-ipv6-multicast = URL points to IPv6 multicast address '{ $host }'
hostname-not-in-allowlist = hostname '{ $host }' is not in the configured allowlist
invalid-mineru-url = invalid MinerU URL '{ $url }': { $source }
mineru-url-scheme-invalid = MinerU URL must use http or https scheme, got '{ $scheme }'
nested-container-depth-limit = nested container exceeds the depth limit
unsupported-inner-format = unsupported inner format
attachment-markdown = > Attachment: `{ $name }` ({ $size } bytes) — { $reason }; content preserved as attachment.
zip-no-convertible-files = *ZIP archive contains no convertible files.*
sqlite-sample-rows-none = Sample rows: *(none)*
sqlite-sample-rows-header = Sample rows (first { $count })

# ── i18n phase 4: remaining converter details ──────────────────────────
pdfium-library-unavailable = pdfium library unavailable
failed-to-read-msg-input = failed to read MSG input: { $source }
msg-extraction-task-failed = MSG extraction task failed: { $source }
msg-compound-file-open-failed = MSG compound file open failed: { $source }
msg-properties-read-failed = MSG properties read failed: { $source }
failed-to-open-epub = failed to open EPUB: { $source }
whisper-api-request-failed = whisper api request failed: { $source }
whisper-api-status = whisper api status: { $source }
whisper-api-decode = whisper api decode: { $source }
failed-reading-response-stream = failed reading response stream: { $source }
response-body-not-utf8 = response body is not valid UTF-8: { $source }
failed-to-parse-json-response = failed to parse JSON response: { $source }
failed-to-fetch-result = failed to fetch result: { $source }
failed-to-read-result = failed to read result: { $source }

# ── catalog.rs self-test (key-set parity guard) ─────────────────────────
test-args = value { $n }
