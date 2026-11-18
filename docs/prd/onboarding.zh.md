# PRD：Agent 接入（Onboarding）

> 状态：现行需求（8 agent 已实现，2026-11-17）；扩列表与 Profiles 为 P5-D 规划（见 [路线图](../roadmap.zh.md)）。
> 事实基准：`v2/crates/onboard/`（22 单测 + 16 golden 合并器测试）。
> 产品定位：数量上我们落后（8 vs magpie 88），但**接入质量**是优势——"接进去不弄坏你原有配置"。竞品调研确认 magpie 部分 agent 走 shell 函数/本地 API 等非配置路径，我们的精确合并口径在配置类 agent 上更严格。

## 1. 背景与目标

用户本机已有多个 coding agent，各自有手工维护的配置（模型、凭据、注释、个性化字段）。接入网关的最大阻力不是"不会配"，而是**怕配坏**。

目标：
1. 自动发现 → 预览 diff → 精确合并 → 真实验证，全程可反悔。
2. 幂等：重复接入不产生副作用。
3. 保留用户的一切：已有字段、TOML 注释、格式。
4. WSL 内的 agent 同等对待。

## 2. 支持范围（现行 8 个）

| Agent | 配置文件 |
|---|---|
| OpenClaw | `~/.openclaw/openclaw.json` |
| Hermes | `~/.hermes/config.yaml` |
| Hermes Flash | `~/.hermes-flash/config.yaml` |
| Claude Code | `~/.claude/settings.json` |
| Codex | `~/.codex/config.toml` + `auth.json` |
| OpenCode | `~/.config/opencode/opencode.json` |
| CodeBuddy | `~/.codebuddy/models.json` |
| WorkBuddy | `~/.workbuddy/models.json` |

P5-D 扩到 ~20（优先 magpie 列表中的主流配置类 agent；shell 函数/本地 API 类接入方式暂不纳入，见 §6）。

## 3. 功能需求

### 3.1 接入流程

| ID | 需求 | 验收 |
|---|---|---|
| R-OB-01 | **detect**：扫描本机已安装/已配置的 agent，只列真实存在的 | 未安装 agent 不出现 |
| R-OB-02 | **plan**：生成改写计划，UI 展示 diff 预览；预览脱敏，token 只出现 `env:`/`file:` 引用 | 预览无明文（R-SEC-04） |
| R-OB-03 | **apply**：用户确认后精确合并——只改模型入口相关字段，其余原样保留 | golden 测试逐 agent 固定 |
| R-OB-04 | 幂等：重复 apply 不产生重复条目/漂移 | 幂等 golden 用例 |
| R-OB-05 | 保留用户已有配置与 TOML 注释（Hermes/Codex/Claude Code 语义由 16 golden 测试固定） | 注释/顺序/未知字段保留断言 |
| R-OB-06 | 写入前 `.bak-<timestamp>` 备份；临时文件 + rename 原子写 | 中途失败不留半文件 |
| R-OB-07 | 路径逃逸校验，不越界写文件 | 恶意路径用例拒绝 |
| R-OB-08 | **verify**：接入后经网关发真实请求验证连通 | verify 失败有明确诊断 |
| R-OB-09 | 断开/还原：可恢复用户原值（备份 + 合并语义支持） | 还原后配置与接入前一致 |

### 3.2 WSL（仅 Windows 宿主）

| ID | 需求 | 验收 |
|---|---|---|
| R-OB-20 | `wsl -l -v` 枚举发行版，UTF-16LE 容错解码 | 异常输出不崩 |
| R-OB-21 | 发行版名白名单 `^[A-Za-z0-9][A-Za-z0-9._-]*$` | 注入名拒绝 |
| R-OB-22 | home 走 UNC：先 `\\wsl.localhost`，回退 `\\wsl$` | 两种前缀用例 |
| R-OB-23 | WSL2 NAT：`ip route show default` 取宿主网关 IP 写入 agent 配置；失败回退 127.0.0.1 并提示 | NAT 场景真实请求可达 |

### 3.3 界面

| ID | 需求 | 验收 |
|---|---|---|
| R-OB-30 | Onboarding 页全流程 detect→plan→apply→verify；Overview 一键配置入口 | UI e2e |
| R-OB-31 | 接入完成推荐 Providers 页（P5-A 后） | 流程衔接 |

## 4. 规划需求（P5-D）

| ID | 需求 | 验收 |
|---|---|---|
| R-OB-40 | agent 列表扩到 ≥20，每个新 agent 配 golden 合并器测试（同 R-OB-03~05 标准） | 列表 ≥20；golden 全覆盖 |
| R-OB-41 | **Profiles（可选）**：一套 agent 配置快照命名保存、一键切换；`/v2/admin/profiles` CRUD + apply | 切换幂等可还原；快照含各 agent 模型选择 |
| R-OB-42 | Profiles 与接入流程复用同一合并器（不引入第二套写入逻辑） | check-deps + 代码审查 |

关联（属 P5-A，见 [路线图 §2.1](../roadmap.zh.md)）：从本机 agent 配置**反向导入** provider（读 base_url/key 引用生成 Provider+Account 草稿）——复用本 crate 的路径与解析知识，方向相反（读而非写）。

## 5. 非功能需求

- 合并器纯逻辑可单测，不依赖真实 agent 安装。
- 所有写入可反悔（备份 + 还原）；失败不静默。
- 凭据边界遵循 [安全 PRD](./security.zh.md)：只传引用。

## 6. 边界

- 不做：shell profile 标记函数、应用本地 API/socket、SQLite 直写等非配置改写接入方式（magpie 有，我们维持配置类口径，控制安全面）。
- 不做：订阅登录态发现（P7 战略议题，见路线图 §7）。
- 不代装 agent（可给安装命令，magpie 同口径）。

## 7. 现状与依赖

- 现行需求全部已实现（M5 交付）；真实客户端回归（Hermes/Codex/Claude Code 经 UI 接入后的真实流式请求）是 GA 前置验证项，需有上游的环境。
- R-OB-41 Profiles 依赖 P5-D 排期决策（路线图列为可选）。