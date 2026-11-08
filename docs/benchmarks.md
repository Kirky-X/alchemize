# Benchmarks — real files (T030)

Measured with `cargo bench --bench bench_real` on this machine
(macOS, Apple silicon, release profile). Each fixture is converted **5 times**
through the full default engine chain; the table reports the **median**
(`min`/`max` in the raw harness output).

Generated: 2026-11-09.

| fixture | format | size | median | throughput | engine |
|---|---|---|---|---|---|
| real.docx | docx (923KB wordlist, 25MB uncompressed XML) | 901KB | 83.4ms | 10.6 MB/s | docx-deep |
| real.xlsx | xlsx (6.7MB morphology lexicon) | 6.5MB | 500.8ms | 12.9 MB/s | xlsx (calamine) |
| real.pdf | pdf (100KB multi-page) | 105KB | 16.3ms | 6.3 MB/s | pdf (pdf-extract) |
| real.odt | odt (ODF text) | 5KB | 0.1ms | 51.9 MB/s | odt-deep |
| real.odp | odp (ODF presentation) | 4KB | 0.1ms | 55.8 MB/s | odp-deep |
| real.pptx | pptx (2 slides) | 49KB | 0.7ms | 66.6 MB/s | pptx-deep |
| real.html | html (101KB page) | 101KB | 1.0ms | 94.4 MB/s | html (htmd) |
| real.rs | rust source (792 lines) | 28KB | 0.0ms | 554.6 MB/s | text (fenced) |
| real.md | markdown | 13KB | 0.0ms | 352.1 MB/s | text (passthrough) |
| real.png | png image | 3KB | 0.0ms | 92.4 MB/s | image |
| real.wav | wav audio (10s speech) | 313KB | 0.0ms | — (metadata mode) | audio |
| real.heic | heic image (512×512) | 9KB | 86.5ms | 0.1 MB/s | heic (sips) |

## Red lines (spec constraints)

| family | limit | result |
|---|---|---|
| docx | < 1 s | 83 ms ✅ |
| xlsx | < 5 s | 501 ms ✅ |
| pdf | < 2 s | 16 ms ✅ |
| plain text / markdown | < 50 ms | < 1 ms ✅ |

## Notes

- `real.heic` is dominated by the `sips` process spawn (~85 ms fixed cost);
  the in-process fallback (no decoder) is sub-millisecond.
- `real.wav`/`real.png` run without vision/ASR providers on this host, so they
  report metadata; audio transcription timing is covered by `vecboost`'s own
  benchmarks.
- The docx number is the deep parser end to end (25 MB of XML event-streamed),
  not the legacy native extractor.
