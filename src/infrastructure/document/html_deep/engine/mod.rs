// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! html-deep 自研引擎 —— 只读 DOM → Markdown 迭代状态机（feature `html-deep`）。
//!
//! 契约（行为规格真源见 specmark 变更 `selfhost-html-engine`，不入库）：
//! - 全程迭代任务栈，DOM 深度不变成 Rust 调用栈深度（万级嵌套安全）；
//! - [`convert`] 不可失败：畸形 DOM（孤儿节点、缺失属性、越界结构）一律静默降级；
//! - 最终输出无尾随换行/空白（调用方按需补 `\n`）；
//! - 用户文本中的 Markdown 语法字符被转义，链接/图片/代码/数学只由引擎自身产出。
//!
//! 模块布局：`output`（片段树）、`writer`（行内管线）、`structure`（元素分类 +
//! 统一 walker）、`machine`（状态机）、`anchors`/`content`/`form`/`media`/
//! `table`/`mathml`/`fallback`/`visibility`（子系统）。

mod anchors;
mod content;
mod fallback;
mod form;
mod machine;
mod math;
mod mathml;
mod media;
mod output;
mod structure;
mod table;
mod visibility;
mod writer;

pub(super) use machine::convert;
