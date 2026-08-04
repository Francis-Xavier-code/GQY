/// 声明式工具注册宏，减少样板代码。
///
/// # 用法
///
/// ## 最简形式（只读，无进度回调）
/// ```ignore
/// declare_tool! {
///     name: "weather",
///     display: ("Weather", "天气查询"),
///     description: "查询天气",
///     parameters: { "type": "object", "properties": { ... } },
///     async fn execute(args: Value) -> Result<String> {
///         // ...
///     }
/// }
/// ```
///
/// ## 完整形式（写权限 + 进度回调）
/// ```ignore
/// declare_tool! {
///     name: "write_file",
///     display: ("Write File", "写入文件"),
///     description: "写入文件内容",
///     permission: writes,
///     parameters: { ... },
///     async fn execute(args: Value, progress: ToolProgress) -> Result<String> {
///         progress.report("writing...");
///         // ...
///     }
/// }
/// ```
///
/// ## 多工具注册
/// ```ignore
/// declare_tools! {
///     tool "hash" {
///         display: ("Hash", "哈希"),
///         description: "计算哈希",
///         parameters: { ... },
///         async fn execute(args) -> Result<String> { ... }
///     },
///     tool "decode" {
///         display: ("Decode", "解码"),
///         description: "解码文本",
///         parameters: { ... },
///         async fn execute(args) -> Result<String> { ... }
///     }
/// }
/// ```
#[macro_export]
macro_rules! declare_tool {
    // 基本形式：只读，无进度
    (
        name: $name:expr,
        display: ($en:expr, $zh:expr),
        description: $desc:expr,
        parameters: $params:tt,
        async fn execute($args:ident: Value) -> Result<String> $body:block
    ) => {
        pub fn register(registry: &mut $crate::tools::ToolRegistry) {
            use $crate::i18n::text as t;
            registry.register(
                $crate::tools::ToolSpec::new(
                    $name,
                    $desc,
                    serde_json::json!($params),
                    |$args| async move $body,
                )
                .display_name(t($en, $zh).to_string()),
            );
        }
    };

    // 写权限形式
    (
        name: $name:expr,
        display: ($en:expr, $zh:expr),
        description: $desc:expr,
        permission: writes,
        parameters: $params:tt,
        async fn execute($args:ident: Value) -> Result<String> $body:block
    ) => {
        pub fn register(registry: &mut $crate::tools::ToolRegistry) {
            use $crate::i18n::text as t;
            registry.register(
                $crate::tools::ToolSpec::new(
                    $name,
                    $desc,
                    serde_json::json!($params),
                    |$args| async move $body,
                )
                .display_name(t($en, $zh).to_string())
                .writes(),
            );
        }
    };

    // 带进度回调的形式
    (
        name: $name:expr,
        display: ($en:expr, $zh:expr),
        description: $desc:expr,
        parameters: $params:tt,
        async fn execute($args:ident: Value, $progress:ident: ToolProgress) -> Result<String> $body:block
    ) => {
        pub fn register(registry: &mut $crate::tools::ToolRegistry) {
            use $crate::i18n::text as t;
            registry.register(
                $crate::tools::ToolSpec::new_with_progress(
                    $name,
                    $desc,
                    serde_json::json!($params),
                    |$args, $progress| async move $body,
                )
                .display_name(t($en, $zh).to_string()),
            );
        }
    };

    // 带进度回调 + 写权限
    (
        name: $name:expr,
        display: ($en:expr, $zh:expr),
        description: $desc:expr,
        permission: writes,
        parameters: $params:tt,
        async fn execute($args:ident: Value, $progress:ident: ToolProgress) -> Result<String> $body:block
    ) => {
        pub fn register(registry: &mut $crate::tools::ToolRegistry) {
            use $crate::i18n::text as t;
            registry.register(
                $crate::tools::ToolSpec::new_with_progress(
                    $name,
                    $desc,
                    serde_json::json!($params),
                    |$args, $progress| async move $body,
                )
                .display_name(t($en, $zh).to_string())
                .writes(),
            );
        }
    };
}

/// 批量注册多个工具的宏。
///
/// # 用法
/// ```ignore
/// declare_tools! {
///     tool "hash" => {
///         display: ("Hash", "哈希"),
///         description: "计算哈希",
///         parameters: { ... },
///         async fn execute(args) -> Result<String> { ... }
///     },
///     tool "decode" => {
///         display: ("Decode", "解码"),
///         description: "解码文本",
///         parameters: { ... },
///         async fn execute(args) -> Result<String> { ... }
///     }
/// }
/// ```
#[macro_export]
macro_rules! declare_tools {
    // 只读批量注册
    (
        $(
            tool $name:literal => {
                display: ($en:expr, $zh:expr),
                description: $desc:expr,
                parameters: $params:tt,
                async fn execute($args:ident: Value) -> Result<String> $body:block
            }
        ),* $(,)?
    ) => {
        pub fn register(registry: &mut $crate::tools::ToolRegistry) {
            use $crate::i18n::text as t;
            $(
                registry.register(
                    $crate::tools::ToolSpec::new(
                        $name,
                        $desc,
                        serde_json::json!($params),
                        |$args| async move $body,
                    )
                    .display_name(t($en, $zh).to_string()),
                );
            )*
        }
    };

    // 带写权限的批量注册
    (
        $(
            tool $name:literal => {
                display: ($en:expr, $zh:expr),
                description: $desc:expr,
                permission: writes,
                parameters: $params:tt,
                async fn execute($args:ident: Value) -> Result<String> $body:block
            }
        ),* $(,)?
    ) => {
        pub fn register(registry: &mut $crate::tools::ToolRegistry) {
            use $crate::i18n::text as t;
            $(
                registry.register(
                    $crate::tools::ToolSpec::new(
                        $name,
                        $desc,
                        serde_json::json!($params),
                        |$args| async move $body,
                    )
                    .display_name(t($en, $zh).to_string())
                    .writes(),
                );
            )*
        }
    };
}
