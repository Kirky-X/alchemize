// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! i18n — 消息国际化门面（无条件依赖，不做 feature 隔离；用户决策
//! 2026-11-11）。消息真源 `locales/{en,zh}/messages.ftl` 编译期内嵌，
//! 经 `sdforge::i18n` 宿主注册表提供查询（库层唯一 i18n 基座）。
//!
//! 消息目录见 [`catalog`]，语言检测见 [`locale`]。调用入口是 crate 根
//! re-export 的 [`tr`](crate::tr) / [`tr_args`](crate::tr_args)。

pub(crate) mod locale;

mod catalog;

mod shorthand;

pub use catalog::{translate, translate_for};
pub use locale::current_locale as locale_of_current_process;
pub use shorthand::{tr, tr_args};
