// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! trait-kit `AsyncKit` 集成层（feature `kit` 门控）。
//!
//! 把 alchemize 的转换门面封装为 trait-kit 模块：[`AlchemizeModule`] 构建
//! 共享门面 `Arc<Alchemize>`（引擎组合随调用方 crate 的 feature 编译态 +
//! [`AlchemizeKitConfig::host_converters`] 注入的宿主引擎），
//! [`ConverterModule`] 依赖它并借出门面句柄，供宿主执行转换与查询引擎面。
//! 核心域 trait 与结构不改动——模块全部手写 `ModuleMeta` +
//! `AsyncAutoBuilder`（0 宏，范式参照 oxcache/src/integrations/kit/
//! module.rs 与 NebulaId/src/sdk/kit.rs）。
//!
//! # 用法
//!
//! ```no_run
//! use alchemize::integrations::kit::{AlchemizeKitConfig, AlchemizeModule, ConverterModule};
//! use alchemize::Input;
//! use trait_kit::prelude::*;
//!
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! let mut kit = AsyncKit::new();
//! kit.set_config(AlchemizeKitConfig::default());
//! kit.register::<AlchemizeModule>()?;
//! kit.register::<ConverterModule>()?;
//! let kit = kit.build().await?;
//! let converters = kit.require::<ConverterModule>()?;
//! let result = converters
//!     .alchemize()
//!     .convert(&Input::bytes(b"hello".to_vec(), "a.txt"))
//!     .await?;
//! # Ok(())
//! # }
//! ```

use std::any::TypeId;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use trait_kit::prelude::*;

use crate::application::ports::{Availability, Converter};
use crate::domain::error::AlchemizeError;
use crate::{Alchemize, AlchemizeBuilder};

/// [`AlchemizeModule`] 构建配置。
///
/// 独立最小结构体（不接入本仓 feature 配置系统）：trait-kit 集成必须是
/// leaf、feature 门控关注点。承载 `AlchemizeBuilder` 的两个常用开关与
/// 宿主引擎注入口；vision/transcribe/MinerU 等依赖注入面直接用
/// [`Alchemize`]/[`AlchemizeBuilder`]（模块借出门面引用，能力不损失）。
///
/// `AsyncKit::config` 要求 `Clone`，而引擎是 `Box<dyn Converter>` 不可克隆：
/// 宿主引擎以工厂闭包（`Arc<dyn Fn() -> Vec<…>>`）承载，每次模块 build
/// 调用一次，闭包内部自行决定引擎构造方式。
#[derive(Clone, Default)]
pub struct AlchemizeKitConfig {
    /// 排除远程（云 API）引擎（对应 `AlchemizeBuilder::with_no_remote`）。
    pub no_remote: bool,
    /// 输入文件大小上限（字节；对应 `AlchemizeBuilder::with_max_file_size`）。
    pub max_file_size: Option<usize>,
    /// 宿主引擎工厂：build 期调用一次，产出的引擎注册进注册表（参与
    /// 优先级路由与 no_remote 排除）。注册发生在 Arc 封装之前，对所有
    /// 分发路径可见。
    pub host_converters: Option<HostConvertersFn>,
}

/// 宿主引擎工厂类型（见 [`AlchemizeKitConfig::host_converters`]）。
pub type HostConvertersFn = std::sync::Arc<dyn Fn() -> Vec<Box<dyn Converter>> + Send + Sync>;

impl std::fmt::Debug for AlchemizeKitConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlchemizeKitConfig")
            .field("no_remote", &self.no_remote)
            .field("max_file_size", &self.max_file_size)
            .field(
                "host_converters",
                &self.host_converters.as_ref().map_or(0, |_| 1),
            )
            .finish()
    }
}

/// 转换门面模块（leaf）：构建共享 `Arc<Alchemize>`。
///
/// 能力组合随调用方 crate 的 feature 编译态（`text`/`pdf`/`docx`/…）——
/// 模块构建不启用任何 feature，编译进哪些引擎注册表就服务哪些格式。
pub struct AlchemizeModule;

impl ModuleMeta for AlchemizeModule {
    const NAME: &'static str = "alchemize";

    fn dependencies() -> &'static [(&'static str, TypeId)] {
        &[]
    }
}

impl AsyncAutoBuilder for AlchemizeModule {
    type Capability = Arc<Alchemize>;
    type Error = AlchemizeError;

    fn build<'a>(
        kit: &'a AsyncKit,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Capability, Self::Error>> + Send + 'a>> {
        Box::pin(async move {
            // 配置可选：未 set_config 时按默认构建（no_remote 关、上限 100 MB、
            // 无宿主引擎）。
            let config: AlchemizeKitConfig = kit.config().unwrap_or_default();
            let mut builder = AlchemizeBuilder::new();
            if config.no_remote {
                builder = builder.with_no_remote();
            }
            if let Some(max) = config.max_file_size {
                builder = builder.with_max_file_size(max);
            }
            for converter in config
                .host_converters
                .map(|factory| factory())
                .unwrap_or_default()
            {
                builder = builder.with_host_converter(converter);
            }
            Ok(Arc::new(builder.build()))
        })
    }
}

/// 转换器句柄：持有共享门面，提供转换执行与引擎面查询。
#[derive(Clone)]
pub struct ConverterHandle {
    alchemize: Arc<Alchemize>,
}

impl ConverterHandle {
    /// 借出共享门面（`convert` / `convert_sync` / `availability` /
    /// `engines` 全量操作可用）。
    pub fn alchemize(&self) -> &Alchemize {
        &self.alchemize
    }

    /// 汇总引擎可用性（`alch doctor` 同款数据面）。
    pub fn availability(&self) -> Vec<(String, Vec<String>, Availability)> {
        self.alchemize.availability()
    }
}

/// 转换器模块：依赖 [`AlchemizeModule`]，借出 [`ConverterHandle`]。
pub struct ConverterModule;

impl ModuleMeta for ConverterModule {
    const NAME: &'static str = "alchemize-converters";

    fn dependencies() -> &'static [(&'static str, TypeId)] {
        static DEPS: std::sync::OnceLock<Vec<(&'static str, TypeId)>> = std::sync::OnceLock::new();
        DEPS.get_or_init(|| vec![(AlchemizeModule::NAME, TypeId::of::<AlchemizeModule>())])
            .as_slice()
    }
}

impl AsyncAutoBuilder for ConverterModule {
    type Capability = ConverterHandle;
    type Error = AlchemizeError;

    fn build<'a>(
        kit: &'a AsyncKit,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Capability, Self::Error>> + Send + 'a>> {
        Box::pin(async move {
            let alchemize = kit.require::<AlchemizeModule>().map_err(|e| {
                AlchemizeError::Other(format!("ConverterModule: require alchemize: {e}"))
            })?;
            Ok(ConverterHandle { alchemize })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::input::Input;
    use crate::domain::result::ConvertResult;

    /// `AlchemizeModule::NAME == "alchemize"` 且为 leaf（无上游依赖）。
    #[test]
    fn alchemize_module_meta() {
        assert_eq!(AlchemizeModule::NAME, "alchemize");
        assert!(AlchemizeModule::dependencies().is_empty());
    }

    /// `ConverterModule::NAME` 与依赖声明（仅依赖根模块）。
    #[test]
    fn converter_module_meta() {
        assert_eq!(ConverterModule::NAME, "alchemize-converters");
        assert_eq!(
            ConverterModule::dependencies(),
            &[(AlchemizeModule::NAME, TypeId::of::<AlchemizeModule>())]
        );
    }

    /// `AlchemizeKitConfig::default()` 语义。
    #[test]
    fn kit_config_defaults() {
        let cfg = AlchemizeKitConfig::default();
        assert!(!cfg.no_remote);
        assert!(cfg.max_file_size.is_none());
        assert!(cfg.host_converters.is_none());
    }

    /// build 快乐路径：register → build → `require::<ConverterModule>()`
    /// → 经句柄完成一次真实文本转换（`text` feature 门控）。
    #[cfg(feature = "text")]
    #[tokio::test]
    async fn converter_module_build_and_convert() {
        let mut kit = AsyncKit::new();
        kit.set_config(AlchemizeKitConfig::default());
        kit.register::<AlchemizeModule>()
            .expect("register alchemize");
        kit.register::<ConverterModule>()
            .expect("register converters");
        let kit = kit.build().await.expect("AsyncKit::build");
        let converters = kit
            .require::<ConverterModule>()
            .expect("require ConverterModule");
        let input = Input::bytes(b"hello world".to_vec(), "note.txt");
        let result = converters
            .alchemize()
            .convert(&input)
            .await
            .expect("convert text");
        assert_eq!(result.markdown, "hello world");
    }

    /// 配置透传：`max_file_size` 经 `AlchemizeKitConfig` 生效（超出上限的
    /// 输入被门面拒绝）。
    #[cfg(feature = "text")]
    #[tokio::test]
    async fn kit_config_is_honored() {
        let mut kit = AsyncKit::new();
        kit.set_config(AlchemizeKitConfig {
            max_file_size: Some(4),
            ..Default::default()
        });
        kit.register::<AlchemizeModule>()
            .expect("register alchemize");
        let kit = kit.build().await.expect("AsyncKit::build");
        let alchemize = kit.require::<AlchemizeModule>().expect("require facade");
        let input = Input::bytes(b"too long input".to_vec(), "note.txt");
        let err = alchemize
            .convert(&input)
            .await
            .expect_err("oversize input must be rejected");
        assert!(
            err.to_string().contains("max_file_size"),
            "error must name the size ceiling: {err}"
        );
    }

    /// 错误分支：未注册依赖时 `ConverterModule::build` 显性失败
    /// （require alchemize 映射为 `AlchemizeError::Other`）。
    #[tokio::test]
    async fn converter_module_without_dependency_errors() {
        let kit = AsyncKit::new();
        let fut = <ConverterModule as AsyncAutoBuilder>::build(&kit);
        let err = match fut.await {
            Ok(_) => panic!("build must fail without the alchemize module"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("require alchemize"),
            "error must name the missing dependency: {err}"
        );
    }

    /// `availability()` 数据面：`text` feature 构建至少注册 text 引擎。
    #[cfg(feature = "text")]
    #[tokio::test]
    async fn availability_lists_text_engine() {
        let mut kit = AsyncKit::new();
        kit.set_config(AlchemizeKitConfig::default());
        kit.register::<AlchemizeModule>()
            .expect("register alchemize");
        kit.register::<ConverterModule>()
            .expect("register converters");
        let kit = kit.build().await.expect("AsyncKit::build");
        let converters = kit.require::<ConverterModule>().expect("require handle");
        let rows = converters.availability();
        assert!(
            rows.iter()
                .any(|(name, _, avail)| name == "text" && *avail == Availability::Available),
            "text engine must be listed available: {rows:?}"
        );
    }

    /// 兜底引擎（测试专用）：把输入原样转成 markdown 占位结果。
    struct PassthroughConverter;

    #[::async_trait::async_trait]
    impl Converter for PassthroughConverter {
        async fn convert(&self, _input: Input) -> Result<ConvertResult, AlchemizeError> {
            Ok(ConvertResult::from_markdown(
                "[passthrough]",
                crate::domain::result::ContentType::Document,
            ))
        }

        fn supported_extensions(&self) -> &[&str] {
            &["txt"]
        }

        fn name(&self) -> &'static str {
            "passthrough"
        }

        fn priority(&self) -> u8 {
            // 高于 TextConverter（10），保证同扩展名竞争时胜出。
            99
        }
    }

    /// 宿主引擎注入：kit 配置携带的引擎接管同扩展名转换（priority 99
    /// 胜过 text 引擎的 10）。
    #[cfg(feature = "text")]
    #[tokio::test]
    async fn host_converter_from_config_takes_over() {
        let mut kit = AsyncKit::new();
        kit.set_config(AlchemizeKitConfig {
            host_converters: Some(std::sync::Arc::new(|| vec![Box::new(PassthroughConverter)])),
            ..Default::default()
        });
        kit.register::<AlchemizeModule>()
            .expect("register alchemize");
        kit.register::<ConverterModule>()
            .expect("register converters");
        let kit = kit.build().await.expect("AsyncKit::build");
        let converters = kit.require::<ConverterModule>().expect("require handle");
        let input = Input::bytes(b"anything".to_vec(), "note.txt");
        let result = converters
            .alchemize()
            .convert(&input)
            .await
            .expect("convert via host engine");
        assert_eq!(result.markdown, "[passthrough]");
        assert!(
            converters.alchemize().engines().contains(&"passthrough"),
            "host engine must be registered"
        );
    }
}
