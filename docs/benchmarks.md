# Benchmarks — real files (T030)

Measured with `cargo bench --bench bench_real` on this machine
(macOS, Apple silicon, release profile). Each fixture is converted **5 times**
through the full default engine chain; the table reports the **median**
(`min`/`max` in the raw harness output).

Generated: 2026-11-09.

| fixture | format | size | median | throughput | engine |
|---|---|---|---|---|---|
| real.docx | docx (923KB wordlist, 25MB uncompressed XML) | 901KB | 75.2ms | 11.7 MB/s | docx-deep |
| real.xlsx | xlsx (6.7MB morphology lexicon) | 6.5MB | 379.8ms | 17.0 MB/s | xlsx (calamine) |
| real.pdf | pdf (100KB multi-page) | 105KB | 12.0ms | 8.6 MB/s | pdf (pdf-extract) |
| real.odt | odt (ODF text) | 5KB | 0.1ms | 49.8 MB/s | odt-deep |
| real.odp | odp (ODF presentation) | 4KB | 0.0ms | 86.9 MB/s | odp-deep |
| real.pptx | pptx (2 slides) | 49KB | 0.7ms | 69.5 MB/s | pptx-deep |
| real.html | html (101KB page) | 101KB | 0.8ms | 124.8 MB/s | html (htmd) |
| real.rs | rust source (792 lines) | 28KB | 0.0ms | 581.9 MB/s | text (fenced) |
| real.md | markdown | 13KB | 0.0ms | 419.6 MB/s | text (passthrough) |
| real.png | png image | 3KB | 0.0ms | 190.7 MB/s | image |
| real.wav | wav audio (10s speech) | 313KB | 0.0ms | — (metadata mode) | audio |
| real.heic | heic image (512×512) | 9KB | 67.4ms | 0.1 MB/s | heic (sips) |

## Red lines (spec constraints)

| family | limit | result |
|---|---|---|
| docx | < 1 s | 75 ms ✅ |
| xlsx | < 5 s | 380 ms ✅ |
| pdf | < 2 s | 12 ms ✅ |
| plain text / markdown | < 50 ms | < 1 ms ✅ |

## Notes

- `real.heic` is dominated by the `sips` process spawn (~65 ms fixed cost);
  the in-process fallback (no decoder) is sub-millisecond.
- `real.wav`/`real.png` run without vision/ASR providers on this host, so they
  report metadata; audio transcription timing is covered by `vecboost`'s own
  benchmarks.
- The docx number is the deep parser end to end (25 MB of XML event-streamed),
  not the legacy native extractor.
