# 网关模型路由：跳过未声明该模型的站点

> 状态：已实现（issue #414 的第一个诉求）。
> 关联：[gateway-provider-compatibility.md](gateway-provider-compatibility.md) §2.9（速度档改写，同一 issue 的第二个诉求）。

## 1. 要解决的问题

用户场景：**同一个模型名，在不同站点上的可得性不同**。

具体案例（issue #414）：用户自己反代的站点能用 `luna`，而市面上的中转站基本不提供 `luna`。他希望在配置里声明「请求 `luna` 时直接走我的反代」，而不是靠故障转移先撞一次错误：

> 虽然现在通过故障切换也能做到，但平白多了一堆错误请求。

现状为什么不够：

| 现有机制 | 为什么不够 |
|---|---|
| **故障转移** | 按 `sort_index` 顺序试，先拿 `luna` 去问中转站，拿到 404 才切换 —— 每次白烧一次错误请求 |
| **模型健康冷却**（`model_health.rs:75`） | 需要**先失败一次**才能学到，且冷却会过期，错误请求周期性复发 |

## 2. 设计

**不引入新的路由配置**。项目里已经有表达「某站点提供哪些模型」的数据：provider 的 `meta.declared_models`（来自 Codex `modelCatalog`，见 `providers::declared_models_from_settings`），聚合模式已经在用它做路由。

把它从「聚合模式专用」提升为**所有模式共用的候选预过滤**即可。

### 2.1 数据

`ProxyGatewaySettings` 新增一个布尔开关：

```rust
/// Skip candidate providers whose declared model catalog does not list the
/// upstream model (issue #414). Off by default.
pub skip_undeclared_models: bool,
```

`#[serde(default)]` 在结构体上，旧存档缺这个键时回退 `false`，向后兼容。

### 2.2 运行时

在 `forward_to_upstream` 的候选循环里，紧邻**已有的健康过滤**加一条同构的过滤（`runtime/upstream.rs`）：

```rust
// 现有：冷却中的候选跳过
if !is_single_provider && !is_model_available(context, &health_key) {
    skipped_by_health.push(provider.name.clone());
    continue;
}

// 新增：没声明这个模型的候选跳过
if !is_single_provider
    && is_undeclared_model_skip(
        settings.skip_undeclared_models,
        any_candidate_declares_models,
        &provider,
        &upstream_model_id,
    )
{
    skipped_by_declaration.push(provider.name.clone());
    continue;
}
```

**「跳过」而非「失败」是核心语义**：与健康过滤完全同构 —— 不计 `attempt_count`、不计健康分、不触发同 provider 重试。因为它不是失败，而是「本来就不该问它」。

### 2.3 三个必须保持的约束

| 约束 | 为什么 |
|---|---|
| 开关**默认 `false`** | 零风险：不主动开启时行为与之前完全一致 |
| 运行时要求**至少一个候选声明了非空目录**（`any_candidate_declares_models`）才启用过滤 | provider 无目录时 `declared_models` 为空。若把「空」读成「什么都不提供」，所有不维护目录的 CLI（Claude/Grok 等，其 `settings_config` 根本没有 `modelCatalog`）会**全量被拒**。这是这个功能最关键的安全阀 |
| 沿用 `is_single_provider` 短路 | 单候选时跳过了就没得用，与健康过滤同理 |

### 2.4 观测与诊断

| 情况 | 响应 |
|---|---|
| 部分候选被跳过 | 静默（这是预期行为）；被跳过的站点记入 `skipped_providers` |
| 全部候选因目录被跳过，且无健康跳过 | 503 `model_not_declared`，`error_category = "model_not_declared"`，`skipped_providers` 列出站点，message 提示「把模型加进某站点的目录，或关掉这个开关」 |
| 全部候选因冷却被跳过 | 503 `model_temporarily_unavailable`（原有行为，额外带 `skipped_undeclared_providers`） |

区分这两种 503 很重要：**目录未命中是用户可修的配置问题，冷却会自己恢复**——不区分就是「神秘的 503」。

### 2.5 匹配键：改写**之后**的模型名

用 `upstream_model_id`（模型改写解析**之后**的值），与聚合模式的路由判定一致（`upstream.rs::aggregate_model_candidates` 同样按 `forwarded` 匹配）。精确相等，不支持通配。

#### 这个键的一个非直觉推论（实现时必须知道）

`declared_models` 的来源是 **`modelCatalog` 映射行 ∪ 该 provider 自己 config **顶层**的默认 `model`**（`codex_declared_models_from_settings` → `aggregate_site_model_specs`，`codex/commands.rs:3655`，复用 `extract_codex_top_level_model`）；而故障转移模式下，Codex 的上游模型恰好被改写成**该 provider 自己的 `default_model`**（`upstream.rs::resolve_upstream_model_id`）。两者同源，于是：

> **凡是设了 config 顶层 `model` 的站点，收到的模型 == 它自己声明的模型，永远不会被本过滤跳过。**

推论：**本过滤实际只能跳过「没设 config 默认模型」的纯透传站点**。

这不是缺陷，而是正确的边界——一个显式配了默认模型的站点，本来就是「我配好了要服务这个模型」，不该被跳过。它同时解释了为什么**不要**改用「客户端请求的模型」做匹配键：那样会让 config 默认模型失去声明效力，反而与故障转移的改写语义打架。

#### 同源判据里的一处真实分歧：`[chat].model`

上面「同源」有个例外，它曾造成一个**真实的误跳**（已修）：

| 提取函数 | 读取顺序 | 用在 |
|---|---|---|
| `codex_model_from_config`（`provider_protocol.rs:181`） | **`[chat].model` 优先**，回退根级 `model` | `model_mapping.default_model` → 改写目标 |
| `extract_codex_top_level_model`（`codex/commands.rs:2637`） | **只读根级 `model`** | `declared_models` |

而 provider 编辑器至今仍把默认模型写成 `[chat]\nmodel = ...`（`CodexPage.tsx:2000`、`:2165`）。于是「只有 `[chat].model`、没有根级 `model`」的站点：**会被改写成 `qwen3-coder`、却不声明 `qwen3-coder`** → 被自己本该服务的请求误跳过。

因此 `is_undeclared_model_skip` 除了查 `declared_models`，**还显式接受该 provider 自己的 `default_model`**：

```rust
let declares = provider.meta.declared_models.iter().any(|d| d == upstream_model_id)
    || provider.model_mapping.default_model.as_deref() == Some(upstream_model_id);
```

**但安全阀 `any_candidate_declares_models` 只认 `declared_models`，不能一并放宽**：每个 provider 都有某种默认模型（Grok 硬编码 `grok-build`，Kimi/Codex 读自己的 config），把 `default_model` 算进安全阀，等于给 Claude/Grok/Kimi 这些**根本不维护目录**的 CLI 重新武装了过滤——正是该安全阀要防的事。

**测试夹具必须遵守**：造「未声明该模型」的站点时，config 里**不能**写 `model = ...`（否则它自己就声明了）。见 §6 的 `insert_codex_catalog_provider(..., config_model: None, ...)`。

## 3. 与现有机制的关系

| 机制 | 关系 |
|---|---|
| **故障转移** | 就是它的一部分——过滤后剩下的候选照常按 `sort_index` 顺序故障转移 |
| **模型健康冷却** | **同构**：两条 `continue` 相邻，都是「跳过不算失败」 |
| **模型改写 `modelRewrites`** | 正交。改写决定「发什么模型名」，本过滤决定「要不要问这个站点」；匹配键是改写后的名字 |
| **速度档改写 `modelServiceTiers`** | 正交 |
| **聚合模式** | 独立。聚合模式有自己的 `declared_models` 路由（更严格：无目录即排除），本开关不介入 |
| **连通性测试** | 走 `provider_override_id` 收窄路径，候选只剩一个 → `is_single_provider` 短路，过滤不生效 |

## 4. 使用前提

**要用这个功能，必须至少给一个站点配模型目录**（通常是用户自己那台能提供该模型的反代）。否则 `any_candidate_declares_models` 为 `false`，开关不生效——这是有意的安全设计，UI 的 help 文案里写明了。

## 5. 实现清单

| # | 层 | 改动 | 文件 |
|---|---|---|---|
| 1 | 数据 | `skip_undeclared_models: bool` + `Default` | `tauri/src/coding/proxy_gateway/types.rs` |
| 2 | 运行时 | `is_undeclared_model_skip()` + `any_candidate_declares_models` 判据 + 过滤 | `tauri/src/coding/proxy_gateway/runtime/upstream.rs` |
| 3 | 诊断 | `model_not_declared` 503 分支 | 同上 |
| 4 | 前端 | 设置面板「韧性」区一个开关（紧邻「模型健康」，同类关注点） | `web/features/settings/pages/GatewaySettingsPanel.tsx` |
| 5 | 前端 | `skip_undeclared_models` 类型 | `web/services/proxyGatewayApi.ts` |
| 6 | i18n | `settings.gateway.fields/hints.skipUndeclaredModels` | `web/i18n/locales/*.json` |
| 7 | 文档 | 本条 + `AGENTS.md` | — |

**零新增迁移、零新增表、零新增 Section。**

## 6. 验证

Rust 单测（`runtime/upstream.rs::tests`）：

- `undeclared_model_skip_engages_only_when_a_catalog_is_maintained` —— 有目录时，未声明的站点被跳过、声明的放行
- `undeclared_model_skip_is_off_when_nobody_maintains_a_catalog` —— **核心安全阀**：无人维护目录时不启用；开关关闭时也不启用
- `undeclared_model_skip_matches_the_declared_id_exactly` —— 精确匹配，近似名不命中
- `undeclared_model_skip_accepts_a_chat_section_default_model` —— **回归测试**：默认模型只写在 `[chat].model` 的站点不被误跳（见 §2.5 的分歧）

端到端集成测试（`runtime.rs::tests`，走完整 `route_request` 链路）：

- `route_request_skips_candidates_that_do_not_declare_the_model` —— 开关开启：未声明的 Mirror 被跳过（上游**没收到任何请求**），Relay 一次成功，`attempt_count == 1` 且 `failover == false`
- `route_request_tries_every_candidate_when_the_filter_is_off` —— 对照组，同一套站点关掉开关：Mirror 先被试、失败后才切 Relay，`attempt_count == 2`
- `route_request_keeps_every_candidate_when_nobody_declares_a_catalog` —— 安全阀端到端：无人维护目录时开关形同不存在
- `route_request_reports_model_not_declared_when_every_candidate_is_filtered` —— 全被跳过 → 503 `model_not_declared`，且**两个上游都没收到请求**

**写夹具的坑**（见 §2.5）：helper 的 `config_model` 参数必须传 `None` 才能造出「未声明」的站点。若传了模型名，该站点就声明了它、也就会被路由到它，过滤永不触发——两个失败测试最初就是踩在这里。

全量：`cargo test --lib`（2813 passed）、`pnpm exec tsc --noEmit`、`pnpm i18n:check`。

## 7. 设计过程中否决的方案

| 方案 | 为什么否决 |
|---|---|
| **全局路由表**（`modelRoutes: [{model, providerId}]`） | 与故障转移语义重叠、新增配置面与 UI；而 `declared_models` 已能表达同样信息 |
| **模型名前缀/后缀标记**（`luna@myrelay`） | 用户无法控制 CLI 发什么模型名——模型名来自 CLI 自己的配置，而网关的卖点正是「接管后不用管 CLI 配置」。且 issue 的诉求是「请求 `luna` 就自动走反代」，不是「请求 `luna@myrelay` 才走」 |
| **严格语义**（空目录 = 不提供任何模型） | 会让所有不维护目录的 CLI 全量被拒。改用「至少一个候选有目录才启用」的数据驱动判据 |

参考实现 [magpie](https://github.com/yetone/magpie) 的 `Group.Rules` 提供了条件-动作分离、只改顺序不删候选、决策可追溯三个思路，但它的 `Group` 抽象主要服务于多账号配额轮转，与本项目「一个 provider 一个 key」的形态不符，未采用。
