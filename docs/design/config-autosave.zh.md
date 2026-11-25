# 配置自动保存与热加载技术方案

> 状态：实施中（2026-11-23）
> 适用范围：`v2/apps/desktop`（Tauri 桌面端）+ `v2/crates/{agent,control,contracts}`。
> 内核冻结：本方案不修改 `v2/crates/{core,decide,proto}`，路由语义不变。

## 1. 背景

桌面端的配置编辑目前是**草稿模式**：

- 三个页面（设置 / 虚拟模型与分组 / 供应商模型设置）共享同一份内存草稿
  （`frontend/src/stores/configDraftStore.ts`）；
- 页面底部有统一的 `DraftActionBar`：先「校验」、再「应用（保存配置）」、或「放弃更改」；
- 应用走 `POST /v2/admin/config/apply`，服务端做**整份文档 CAS**（`If-Match` = 磁盘
  revision）+ 原子写 + 备份，并返回 `restart_required: true`——**内核不做热加载**。

这套模型对"结构性大改"（重建虚拟模型池、批量导入）是合适的，但对日常调参是摩擦：
改一个开关要经历「校验 → 应用 → 重启网关 → 回来继续改」。目标形态是**改完即生效**。

## 2. 目标与非目标

### 目标

1. 日常编辑（单值、候选级字段、组内参数）**自动保存**，页面不再出现「校验 / 应用」按钮；
2. 自动保存必须**不破坏配置合法性**：绝不把本地校验不通过的中间态写盘；
3. 自动保存必须**不被并发写者打死**：provider compile、另一个窗口、外部编辑都可能改
   磁盘配置，409 要能静默 rebase，而不是弹冲突面板打断输入；
4. 保存后**立即生效**：`apply` 成功后热替换运行时目录，`restart_required` 只在真正
   需要重启的字段变化时为 true；
5. 保留回滚能力：高频保存不能把备份环形烧穿。

### 非目标

- 不改变内核路由语义（`core/decide/proto` diff = 0）；
- 不做多用户协作/实时合并服务端；
- 不把密钥写入纳入自动保存（明文 key 仍走显式表单 + 引用）；
- 不改变 `GET /config` 的脱敏语义；
- 不做草稿跨进程持久化（自动保存后"关闭即丢"的问题消失，持久化失去必要性）。

## 3. 现状事实（实施前）

| 事实 | 位置 |
|---|---|
| 草稿基准 `baseRevision` 只在创建/上次成功应用时定格，apply 用它做 `If-Match` | `frontend/src/stores/configDraftStore.ts` |
| 脏草稿 + 服务端新 revision → 直接进冲突态 | 同上 `ensureDraft` |
| apply 是整份 TOML 的 CAS 写盘，写前备份，返回 `restart_required: true` | `crates/control/src/config_service.rs` |
| 备份保留上限 10，每次写盘都备份 | `config_service.rs` `MAX_BACKUPS` |
| `Pipeline` 持有 `Arc<Catalog>`，判官/组判官/成本闸在构造时固化 | `crates/agent/src/pipeline.rs` |
| control 与 server 共用同一个 `Arc<Pipeline>` | `crates/server/src/main.rs` |
| 配置写锁 `config_write_lock` 串行化 apply / rollback / compile | `crates/control/src/state.rs` |

## 4. 总体方案

分三层，职责不重叠：

```
编辑层（页面）          保存层（前端引擎）              生效层（服务端）
─────────────          ─────────────────              ────────────────
字段提交事件    ──▶     防抖合并 + 合法性门   ──▶      config/apply（CAS 写盘）
（blur / change）              │                          │
                               │ 422 ─▶ 待修复             ├─ 写前备份（节流）
                               │ 409 ─▶ 三方 rebase        └─ 成功 ─▶ Pipeline 热替换
                               │ 其它 ─▶ 重试 / 离线                 （catalog / judge /
                               └─ 成功 ─▶ 已保存                      group_judges / cost_gate）
```

设计原则：**草稿仍是唯一编辑缓冲，自动保存只是把"何时提交"从按钮改成事件 + 防抖。**
状态机、revision 语义、冲突出路全部保留，删除的只是"校验"这一手动步骤。

## 5. 保存语义

### 5.1 状态机

| 状态 | 含义 | 进入条件 | 出路 |
|---|---|---|---|
| `saved` | 已保存且与磁盘一致 | 初始 / apply 成功 | 编辑 → `pending` |
| `pending` | 有改动，等待提交 | 编辑发生 | 防抖到期 → `saving` |
| `saving` | apply 在途 | 提交开始 | 成功 → `saved`；失败 → 见 5.3 |
| `invalid` | 有本地校验问题，不提交 | 提交前本地 issues 非空 | 修好 → `pending` |
| `conflict` | 与磁盘改动重叠，需人工选择 | rebase 失败（见 6.3） | 保留我的 / 用最新的 |
| `reloading` | 已写盘，运行中目录替换中 | apply 成功后 reload | 完成 → `saved` |

`DraftActionBar` 被替换为只读的保存状态指示（`ConfigSaveBar`），"放弃更改"改为
"还原到已保存"（refetch + reset）。`DraftStatusBar` 保留为全局状态条，并在有未保存
改动时提供同一个还原入口。

### 5.2 提交边界

| 控件 | 提交时机 |
|---|---|
| 文本 / 数字输入 | `blur`（失焦）才提交；输入过程中只更新草稿并标记 `pending` |
| select / switch / chip / 滑块 | 立即提交 |
| 结构操作（新建/删除组、候选池增删） | 立即提交；删除类操作保留二次确认 |
| 导入 provider / 配置回滚 / 明文密钥 / 整份 TOML | **不自动保存**，保持显式按钮 |

实现方式：保存引擎监听全局 `focusout`。当 `document.activeElement` 是文本类输入时，
防抖到期也不提交，只把"待提交"标记留着；失焦事件触发立即提交。这样不需要给每个
输入框手写 `onBlur`，也不会出现 `server.listen` 被写成 `17` 的中间态。

防抖窗口 800ms；连续结构操作合并为一次 apply。

### 5.3 错误处理

| 错误 | 处理 |
|---|---|
| 本地 issues | 不提交，状态 `invalid`，页面标红不变；修好后自动重试 |
| `422 config_invalid`（服务端 issues） | 灌入 `serverIssues`，状态 `invalid`；不弹窗 |
| `409 config_conflict` | 进入 rebase（§6.3）；失败才 `conflict` |
| 网络 / 5xx | 指数退避重试（1s/2s/4s，最多 3 次），仍失败 → `conflict` 面板提示"保存失败"并保留草稿 |
| 关闭窗口 | 先 flush；有未保存或保存失败 → 拦截关闭（复用 `draftAllowsClose`） |

### 5.4 与服务端 revision 的关系

- `baseRevision` 依旧是 apply 的 `If-Match`，语义不变；
- apply 成功后 `commitApply(revision)` 把基准重建为已提交内容（现状逻辑可复用）；
- query 重取同 revision 幂等；
- 编辑中收到新 revision：先不动草稿；提交时按 §6.3 处理（不再在 `ensureDraft` 里
  直接进冲突态——自动保存下"新 revision + 脏草稿"是常态，不能一撞就停）。

## 6. 并发与冲突

### 6.1 提交前探测（廉价路径）

自动提交前先 `GET /v2/admin/config` 取当前 revision：

- 与 `baseRevision` 相同 → 直接 apply；
- 不同 → 走 §6.3 rebase；
- GET 失败 → 用已知 `baseRevision` 直接 apply，让 409 兜底。

### 6.2 为什么必须 rebase

自动保存把"编辑窗口"拉长到用户操作全程。以下都会改磁盘 revision：

- provider compile（勾选模型、改账号、改模型元数据后重新编译候选）；
- 另一台窗口 / CLI；
- 后台导入。

如果维持"脏草稿 + 新 revision = 冲突"的现状，保存会被频繁阻塞。冲突应当只在
**真正重叠的字段**上发生。

### 6.3 三方 rebase 算法

输入：`base`（baseRevision 对应文档）、`mine`（我的草稿）、`theirs`（磁盘最新文档）。

1. `mineΔ = diffDrafts(base → mine)`，`theirsΔ = diffDrafts(base → theirs)`，均为路径集合
   （数组按元素 `id` / `model_id` 对齐，例如 `candidates[gpt-5]`、`virtualModels[auto]`）；
2. 若 `mineΔ ∩ theirsΔ = ∅`：构造 `merged = theirs + mineΔ`（只把我改的路径写回
   `theirs`），`baseRevision = theirs.revision`，用 `merged` 提交一次（静默）；
3. 若交集非空：进入 `conflict`，展示重叠字段路径与双方取值，提供两种出路：
   - **保留我的**：同上 merge，但重叠字段取 mine（显式覆盖）；
   - **用最新的**：`resetFromDocument(theirs)`，丢弃我的草稿。

合并是确定性的纯函数（`lib/config-draft-merge.ts`），可单测。数组元素按 id 对齐；
id 本身被改（重命名组）视为重叠冲突，不做猜测。

### 6.4 同源写者

provider compile 与 apply 共用 `config_write_lock`，不会交错写；rebase 只需处理
"我基于旧基准"的情况，不需要跨进程锁。

## 7. 生效：热加载

### 7.1 运行时目录交换

`Pipeline` 现状把 `catalog / judge / group_judges / cost_gate` 作为不可变字段。改为：

```rust
struct PipelineRuntime {
    catalog: Arc<Catalog>,
    judge: Arc<JudgeRuntime>,
    group_judges: HashMap<VirtualModelId, Arc<GroupJudgeRuntime>>,
    cost_gate: CostGateMode,
}

pub struct Pipeline {
    db: Arc<Db>,
    runtime: RwLock<Arc<PipelineRuntime>>,   // 读多写少；请求开始时取一次快照
    upstream: HttpUpstream,
    shadow_tx: Sender<ShadowRecord>,
    warned_sessions: Mutex<BTreeSet<String>>,
}
```

- `Pipeline::catalog()` 返回 `Arc<Catalog>`（原来 `&Catalog`），调用方零拷贝持有快照；
- `Pipeline::reload(catalog)` 重新构建 judge / group_judges / cost_gate，整体 `mem::replace`；
  在途请求继续用它开始时的快照，语义与"重启前已开始的请求"一致；
- `warned_sessions` 不随 reload 重置（避免重复告警）；
- judge 构建失败沿用启动时的降级策略（off + 日志），不阻断 reload。

### 7.2 端点

`POST /v2/admin/config/reload`（`AdminWrite`）：

- 读磁盘配置 → `into_catalog()` → `pipeline.reload()`；
- 返回 `ConfigReloadResponse { revision, reloaded, restart_required, issues }`；
- 磁盘配置非法 → `422 config_invalid` + issues（不替换运行时，服务保持可用）。

`apply` / `rollback` 在两个 handler 内**自动 reload**（同进程，无需外部触发）；
SSE `config.applied` 的 `restart_required` 改为真实值。provider compile 写盘后同样
reload，使"勾选模型 → 候选生效"不再需要重启。

### 7.3 供应商选择即时编译（候选与目录不再错位）

组页「添加模型」的可选列表来自 `config.toml` 的 `[[candidates]]`（编译产物），而供应商页的
勾选状态存在 SQLite。以前两者只在手动点「应用模型选择」时才同步，于是出现"供应商页勾了、
组页看不到"的错位。

现在 `catalog/select`、provider/account CRUD、auto-include 之后服务端立即重编译候选
（`compile_after_provider_change`，尽力而为：失败只记录日志，不使本次编辑失败）并热加载。
重编译会继承上一版同 id 候选的**候选级设置**（上游 model / 协议 / 输入模态 / 思考档 /
请求头），勾选其他模型不会把这些设置抹掉。

取消勾选 / 删除账号导致某候选不再产生时：若它**仍被组候选池或规则引用**，保留为
`enabled = false` 的停用候选，而不是直接删除——否则 `candidate_pool` 悬空会让整份配置
校验失败，编译永远失败（表现就是"勾选变了、组页候选不变"）。停用候选不参与路由；
引用清掉后，下一次编译会把它移除。

### 7.4 `restart_required` 判定

只有 `[server]` 段变化才需要进程重启（监听地址、鉴权、TLS 在进程启动时固化）。
判定在写盘路径完成：比较写前/写后 `server` 段的序列化值；变化 → `restart_required: true`，
否则 `false`。其余段落全部热加载。

桌面端在 `restart_required: true` 时显示"已保存，需重启网关"并提供一键重启
（`supervisor.restart_with_backoff`），自动保存本身不会反复触发重启。

## 8. 备份节流

现状：每次 apply 写一份 `config.toml.bak-<micros>`，保留 10 份。自动保存下十分钟就能
把回滚历史全冲掉。改为**编辑会话合并**：

- 距上一份备份 < 5 分钟且存在备份 → 不再新建，直接复用最近一份（它代表本次会话开始
  前的状态）；
- 距上一份备份 ≥ 5 分钟 → 新建一份；
- 上限从 10 提到 20（按天回滚的场景仍可用）；
- `rollback` 语义不变：总是还原最近一份备份（即"最近一次编辑会话之前"的状态）。

规则是时间窗口，不依赖进程内状态，CLI / 桌面 / 外部调用一致。

## 9. 契约变更

新增端点（既有 DTO 字段不动）：

```rust
/// POST /v2/admin/config/reload 响应
pub struct ConfigReloadResponse {
    pub revision: String,
    pub reloaded: bool,
    pub restart_required: bool,
    pub issues: Vec<ConfigIssue>,
}
```

`ApplyResponse.restart_required` 字段不变，取值语义由"恒为 true"改为"是否真的需要重启"。
`ApplyRequest.restart`（未使用）保留，不改。

`ApplyResponse` 之外，`config/apply` 的 SSE payload 同步改为真实值。

## 10. 文件清单

### 后端

| 文件 | 变更 |
|---|---|
| `crates/agent/src/pipeline.rs` | `PipelineRuntime` + `RwLock` 快照 + `reload()`；内部改为每请求取一次快照 |
| `crates/agent/src/lib.rs` | 如需导出 reload 结果类型 |
| `crates/control/src/config_service.rs` | `ApplyOutcome.restart_required`；备份节流；`reload_runtime()` |
| `crates/control/src/lib.rs` | `config_apply` / `config_rollback` 后自动 reload；新增 `config_reload` 路由；SSE 真实值 |
| `crates/control/src/provider_service.rs` | compile 写盘后 reload |
| `crates/contracts/src/admin.rs` | `ConfigReloadResponse` |
| `crates/contracts/bindings/admin.ts` | 重新生成 |
| `docs/contracts/admin-api.zh.md` | 新端点与语义 |

### 前端

| 文件 | 变更 |
|---|---|
| `src/lib/config-autosave.ts` | 纯函数：合并窗口、提交时机判定、rebase 规划 |
| `src/lib/config-draft-merge.ts` | 三方合并（路径级，数组按 id 对齐） |
| `src/stores/configDraftStore.ts` | 新状态 `pending/invalid/saved` 语义；不再在 `ensureDraft` 直接冲突 |
| `src/hooks/useConfigDraft.ts` | 暴露保存状态与 `flush`；删除 validate 手动路径 |
| `src/hooks/useAutosave.ts`（新） | 单例调度器：防抖、focusout、GET 探测、rebase、重试 |
| `src/components/config/ConfigSaveBar.tsx`（新） | 保存状态 + 还原入口；替换 DraftActionBar |
| `src/components/config/DraftStatusBar.tsx` | 接新状态，提供还原按钮 |
| `src/pages/SettingsPage.tsx` / `VirtualModelsPage.tsx` / `providers/ModelSettingsPanel.tsx` | 移除 DraftActionBar；字段提交边界 |
| `src/api/client.ts` / `queries/admin.ts` | reload 调用；apply 返回真实 `restart_required` |
| `src/api/types.ts` | 新 DTO |

## 11. 测试计划

| 层 | 用例 |
|---|---|
| agent | reload 后新候选可被选中；reload 期间在途请求继续用旧快照；judge 构建失败降级 |
| control | apply 后 criteria 立即生效（不发新请求读 catalog）；`server.listen` 变化 → `restart_required`；备份节流窗口内只有一份备份；reload 端点 422 时不替换运行时 |
| 前端（node --test） | 防抖合并、focusout 立即提交、本地 issues 不提交、rebase 无交集合并、rebase 有交集冲突、422 issues 落状态 |
| 手工 | 设置页改监听端口（重启提示）；勾选模型（候选即时生效）；两个窗口同时改不同字段（静默合并）；同时改同一字段（冲突面板） |

门禁：`cargo test`（tr-agent/tr-control/tr-contracts）、`cargo clippy -D warnings`、
`cargo xtask contract-check`、`npx tsc -b`、`npm run check:uno`、`npm run build`。

## 12. 风险与回退

| 风险 | 缓解 |
|---|---|
| 自动保存写盘频率高，SSD / 审计表膨胀 | 800ms 防抖 + 会话备份合并；apply 审计行可后续按会话聚合 |
| 热替换 catalog 与在途请求不一致 | 每请求取一次快照，请求内自洽；语义等同重启前进入的请求 |
| rebase 合并错误导致配置被改坏 | 合并是路径级白名单（只写 mineΔ），冲突走人工；写前服务端仍全量校验 |
| 自动保存把非法值写盘 | 本地 issues 门 + 服务端 422；绝不以 422 响应替代校验 |
| `restart_required` 语义变化影响 CLI/脚本 | CLI 只打印该值；文档同步更新；旧脚本看到 false 只会更宽松 |

回退：前端删掉自动保存调度器、恢复 `DraftActionBar` 即可回到草稿模式；后端 reload
端点保留，`apply` 的自动 reload 可单独回退为返回 `restart_required: true`。

## 13. 实施顺序

1. 本文档 + 契约文档更新；
2. 后端：`PipelineRuntime` + reload + `config/reload` + apply/rollback/compile 自动 reload + 备份节流；
3. 前端：autosave 调度器 + 状态机 + 三页接线 + 保存状态 UI；
4. 单测与门禁；
5. 桌面端启动验证：改一个设置 → 状态条显示"已保存" → 网关不重启即生效。
