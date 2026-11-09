# lopdf vendor fork（alchemize）

## 来源

- 上游：[J-F-Liu/lopdf](https://github.com/J-F-Liu/lopdf)，crates.io `lopdf 0.42.0`
- 许可：MIT（`LICENSE` 随包携带）
- 快照：crates.io 发布源码原样 vendor，无任何删改入库；本 fork 增量见下

## 增量（相对上游 0.42.0）

**解压炸弹防护：流解压输出上限。** 上游 `Stream::decompressed_content()` 用
`read_to_end` / `decode_all` 无上限展开 FlateDecode / LZWDecode 流——几 KB 的
压缩流可膨胀到 GB 级内存（CVE 类 decompression bomb）。alchemize 用
pdf-extract 0.12 解析不可信 PDF，而 pdf-extract 钉死 lopdf 0.42 且无新版
（0.47 才在上游引入 `extract_text_with_limit`），等不到上游接口，故就地
vendor 并在解压器层加界：

1. `src/lib.rs`：`MAX_STREAM_DECOMPRESSED_BYTES` 静态项（默认 64 MiB，对齐
   alchemize 容器族 `MAX_ENTRY_BYTES`）+ `set_max_stream_decompressed_bytes`
   （0 = 不设限，供下游显式放宽）。
2. `src/error.rs`：`DecompressError::MemoryLimitExceeded { limit }`。
3. `src/object.rs`：
   - `decompress_zlib`：`read_to_end` 换成手动有界读取循环（raw-deflate
     回退路径同样有界）；超限返回 `MemoryLimitExceeded`。
   - `decompress_lzw_loop`：`decode_all` 输出经 `CappedWriter` 有界写入，
     超限返回 `MemoryLimitExceeded`（签名 `Vec<u8>` → `Result<Vec<u8>>`，
     私有函数无兼容负担）。
   - `decompress_predictor`：`Colors`/`BitsPerComponent` 范围护栏 +
     `checked_mul`，防止恶意 DecodeParms 造出行宽乘法溢出。
4. `src/lib.rs` 违规标记（线程本位）：`stream_decompression_limit_hit` /
   `reset_stream_decompression_limit_hit`——部分消费者（pdf-extract 的
   `get_contents`）会吞掉单流解压错误并回退原始压缩字节，宿主转换后读该
   标记即可显性拒绝，避免炸弹变成静默垃圾输出。

单点收口：parse 阶段的 ObjStm/XRefStm（`reader.rs` / `parser_aux.rs`）与
pdf-extract 的内容流、ICC 流、字体流全部经 `Stream::decompressed_content()`
走到上述两个解压函数，无旁路。

## 测试 fixture（fork 自身测试用）

上游 `include_bytes!` 引用的资产不在 crates.io tarball 内，fork 补了最小
fixture：`assets/example.pdf`（手工构造的最小 PDF 1.5）与
`tests/resources/fonts/Montserrat-Regular.ttf`（160 字节手工最小 TTF，
head/hhea/maxp 三表，仅满足结构测试，非真字体）。

## 接入

主 workspace `Cargo.toml` 的 `[patch.crates-io]`：
`lopdf = { path = "third_party/lopdf" }`，pdf-extract 的 `lopdf 0.42` 依赖
整体重定向到本 fork。

## 上游路线

升级到 lopdf ≥ 0.43/0.47（自带 per-page `max_decompressed_size` 接口）或
pdf-extract 发布接上限接口的新版后，本 fork 退役、恢复 crates.io 直连。
若上游愿意收 cap 补丁，本 fork 的 object.rs 改动可直接作为 patch 草稿。
