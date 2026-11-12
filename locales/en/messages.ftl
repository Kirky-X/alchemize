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

# ── infrastructure/remote/mineru ────────────────────────────────────────
mineru-task-submitted = MinerU task submitted
mineru-task-completed = MinerU task completed
unknown-task-status = unknown task status

# ── catalog.rs self-test (key-set parity guard) ─────────────────────────
test-args = value { $n }
