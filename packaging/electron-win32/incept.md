# Token Router DLL 接入指南

> 面向 Electron 宿主（OpenCode / Codex / FlowyClaw 等）。按步骤创建下列文件即可接入：FFI 启动 Gateway → Admin HTTP 配置上游 → IPC 暴露给 UI → Agent 指向 `http://127.0.0.1:<port>/v1`（示例端口 `11080`）。
>
> 参考实现：
> - **Electron + FFI**：FlowyClaw 项目 `electron/utils/flowy-router-*.ts`
> - **Tauri + Rust crate**：本仓库 [`desktop/`](./desktop/)（`embedded::start`，无需 DLL）
>
> HTML 版见 [incept.html](./incept.html)。一键打包接入文件：`make package-electron-win`（输出 `target/dist/token-router-electron-win32-x64/`）。

---

## 目录

0. [构建 DLL 并放入 resources](#0-构建-dll-并放入-resources)
1. [package.json 依赖](#1-packagejson-依赖)
2. [token-router-ffi.ts — FFI 绑定](#2-electronutilstoken-router-ffits--ffi-绑定)
3. [token-router-service.ts — 启动编排](#3-electronutilstoken-router-servicets--启动编排)
3.1. [端云上游配置与 POST /v1/admin/setup](#31-端云上游配置与-post-v1adminsetup)
4. [ipc-handlers.ts — 主进程 IPC](#4-主进程-ipc-handler)
5. [ipc-channels.ts + preload — 白名单与暴露](#5-preload-白名单--暴露-api)
6. [渲染进程调用 + OpenClaw Provider](#6-渲染进程调用--openclaw-provider)
7. [vite.config + electron-builder 打包](#7-viteconfig--electron-builder-打包)
8. [应用退出清理](#8-应用退出清理)
9. [koffi 冒烟测试（可选）](#9-koffi-冒烟测试可选)
10. [验证命令](#10-验证命令)
11. [Tauri / Rust crate 嵌入（可选）](#11-tauri--rust-crate-嵌入可选)
12. [Gateway 发现（Status Pipe）](#12-gateway-发现status-pipe)

---

## 目标文件树

```
your-electron-app/
├── resources/
│   └── win32/x64/token_router.dll      ← Step 0
├── electron/
│   ├── utils/
│   │   ├── token-router-ffi.ts         ← Step 2
│   │   └── token-router-service.ts     ← Step 3
│   ├── main/ipc-handlers.ts            ← Step 4 追加 handler
│   ├── flowy/ipc-channels.ts           ← Step 5 白名单
│   └── preload/index.ts                ← Step 5 暴露 API
├── src/.../ModelSelect.tsx             ← Step 6 渲染进程调用
├── vite.config.ts                      ← Step 7 external ffi-rs
├── electron-builder.yml                ← Step 7 extraResources
└── package.json                        ← Step 1
```

**调用链：**

```
UI invokeIpc('token-router:ensureStarted')
  → 主进程 ensureTokenRouterStarted()
  → FFI token_router_start(homeDir, port)   // home 与 port 必填
  → 轮询 GET /v1/admin/status
  → POST /v1/admin/setup
  → 返回 { url: 'http://127.0.0.1:11080/v1' }
  → 写入 OpenClaw Provider baseUrl
```

**设计要点**

| 层次 | 职责 |
|------|------|
| **FFI** | Gateway 生命周期（启动 / 停止 / 状态 / 监听地址） |
| **Admin HTTP** | 上游配置热更新、状态、统计（`/v1/admin/*`） |
| **Chat HTTP** | Agent 入口（`/v1/chat/completions`、`/v1/responses`、`/anthropic` 等） |

### 三种嵌入方式

| 方式 | 启动入口 | 适用宿主 | 数据目录 |
|------|----------|----------|----------|
| **CLI 守护进程** | `token-router gateway start` | 服务器、开发机 | `~/.token-router/`（默认） |
| **DLL / FFI** | `token_router_start(home, port)` | Electron 主进程 | 由 `home_dir` 指定 |
| **Rust crate** | `embedded::start(home, port)` | Tauri / 自研 Rust 壳 | `None` 时用默认 home；桌面版见 `~/.token-router-desktop/` |

> **端口说明**：CLI 默认监听 `16621`；Electron / 桌面示例常用 `11080`。FFI **必须**显式传入 `home_dir` 与 `port`（无默认值）。

### Admin HTTP 接口（节选）

| 路径 | 方法 | 说明 |
|------|------|------|
| `/health` | GET | 健康检查 |
| `/v1/admin/status` | GET | 监听地址、版本 |
| `/v1/admin/setup` | GET / POST | 读取 / 热更新上游配置 |
| `/v1/admin/setup/init` | POST | 初始化默认配置 |
| `/v1/admin/stats` | GET | 路由统计（`?scope=session\|global`） |
| `/v1/admin/routing-logs` | GET | 路由决策日志 |
| `/v1/admin/auth-keys` | GET / POST | API Key 管理 |
| `/v1/admin/shutdown` | POST | 关闭 Gateway |
| `/v1/admin/restart` | POST | 重启 Gateway |
| `/setup` | GET | Web 配置页（浏览器） |

若 `config.toml` 配置了 `gateway.admin_token`，Admin 请求须带 `X-Token-Router-Admin-Token`。

---

## 0. 构建 DLL 并放入 resources

在 **token-router** 仓库执行：

```bash
make release-dylib
# Windows 产物: target/release/token_router.dll

# 或一键打包 Electron 接入所需全部文件（含 zip）:
make package-electron-win
# 输出: target/dist/token-router-electron-win32-x64/
#       target/dist/token-router-electron-win32-x64.zip

mkdir -p your-app/resources/win32/x64
cp target/release/token_router.dll your-app/resources/win32/x64/
```

| 平台 | 文件名 | resources 路径 |
|------|--------|------------------|
| Windows x64 | `token_router.dll` | `resources/win32/x64/` |
| macOS arm64 | `libtoken_router.dylib` | `resources/darwin/arm64/` |
| Linux x64 | `libtoken_router.so` | `resources/linux/x64/` |

C 头文件：`ffi/token_router.h`。`Cargo.toml` 中 `crate-type = ["rlib", "cdylib", "staticlib"]`。

### C ABI 导出函数

| 函数 | 返回值 | 说明 |
|------|--------|------|
| `token_router_version()` | `const char*` | 库版本（静态字符串，勿 free） |
| `token_router_start(home_dir, port, error_out, error_out_len)` | `int32` | 后台启动；`home_dir` 与 `port` **必填**（无默认值） |
| `token_router_stop(error_out, error_out_len)` | `int32` | 停止并等待线程退出 |
| `token_router_is_running()` | `int32` | 运行中返回 `1`，否则 `0` |
| `token_router_gateway_url(url_out, url_out_len)` | `int32` | 写入 `http://host:port`；失败返回负错误码 |

| 状态码 | 值 | 含义 |
|--------|-----|------|
| `TOKEN_OK` | 0 | 成功 |
| `TOKEN_ERR_ALREADY_RUNNING` | 1 | 重复 start |
| `TOKEN_ERR_NOT_RUNNING` | 2 | 未运行 |
| `TOKEN_ERR_INVALID_ARG` | 3 | 参数非法 |
| `TOKEN_ERR_INTERNAL` | 4 | 内部错误 |

数据目录由 `home_dir` 指定（配置固定为 `{home}/config.toml`）。**FFI 不提供默认 home/port**；CLI 默认 home 为 `~/.token-router/`，默认 port 为 `16621`（仅 `gateway start`/`restart` 时生效）。

---

## 1. package.json 依赖

```json
{
  "dependencies": {
    "ffi-rs": "1.3.1"
  },
  "optionalDependencies": {
    "@yuuang/ffi-rs-win32-x64-msvc": "1.3.1"
  }
}
```

---

## 2. electron/utils/token-router-ffi.ts — FFI 绑定

用 ffi-rs 加载 DLL，封装 C ABI。

- **打包后路径**：`process.resourcesPath/resources/{platform}/{arch}/token_router.dll`
- **开发路径**：`process.cwd()/resources/{platform}/{arch}/token_router.dll`

```ts
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { DataType, close, load, open } from 'ffi-rs';

const LIBRARY_NAME = 'token_router';
const ERROR_BUFFER_LEN = 4096;
const URL_BUFFER_LEN = 2048;

export enum TokenRouterStatusCode {
  OK = 0,
  AlreadyRunning = 1,
  NotRunning = 2,
  InvalidArg = 3,
  Internal = 4,
}

function cStringFromBuffer(buffer: Buffer): string {
  const nulIndex = buffer.indexOf(0);
  const end = nulIndex >= 0 ? nulIndex : buffer.length;
  return buffer.subarray(0, end).toString('utf8');
}

function dylibFileName(): string {
  if (process.platform === 'win32') return 'token_router.dll';
  if (process.platform === 'darwin') return 'libtoken_router.dylib';
  return 'libtoken_router.so';
}

function defaultRouterDllPath(): string {
  const rel = join(process.platform, process.arch, dylibFileName());
  const packaged = process.resourcesPath
    ? join(process.resourcesPath, 'resources', rel)
    : '';
  if (packaged && existsSync(packaged)) return packaged;
  return join(process.cwd(), 'resources', rel);
}

export class TokenRouterFfiBinding {
  private opened = false;
  constructor(private readonly dllPath = defaultRouterDllPath()) {}

  start(homeDir: string, port: number) {
    if (!homeDir || !port) {
      throw new Error('homeDir and port are required');
    }
    const errorOut = Buffer.alloc(ERROR_BUFFER_LEN);
    const code = this.callStatus(
      'token_router_start',
      [DataType.String, DataType.U16, DataType.U8Array, DataType.U64],
      [homeDir, port, errorOut, errorOut.length],
    );
    if (code !== TokenRouterStatusCode.OK) {
      throw new Error(cStringFromBuffer(errorOut) || 'token_router_start failed');
    }
  }

  stop() {
    const errorOut = Buffer.alloc(ERROR_BUFFER_LEN);
    const code = this.callStatus(
      'token_router_stop',
      [DataType.U8Array, DataType.U64],
      [errorOut, errorOut.length],
    );
    if (code !== TokenRouterStatusCode.OK) {
      throw new Error(cStringFromBuffer(errorOut) || 'token_router_stop failed');
    }
  }

  isRunning(): boolean {
    return this.callStatus('token_router_is_running', []) === 1;
  }

  gatewayUrl(): string {
    const urlOut = Buffer.alloc(URL_BUFFER_LEN);
    const n = this.callStatus(
      'token_router_gateway_url',
      [DataType.U8Array, DataType.U64],
      [urlOut, urlOut.length],
    ) as number;
    if (n < 0) throw new Error(cStringFromBuffer(urlOut) || 'gateway_url failed');
    return cStringFromBuffer(urlOut);
  }

  version(): string {
    return this.call('token_router_version', DataType.String, [], []) as string;
  }

  close() {
    if (this.opened) {
      close(LIBRARY_NAME);
      this.opened = false;
    }
  }

  private ensureOpen() {
    if (this.opened) return;
    if (!existsSync(this.dllPath)) throw new Error(`DLL not found: ${this.dllPath}`);
    open({ library: LIBRARY_NAME, path: this.dllPath });
    this.opened = true;
  }

  private call(func: string, ret: DataType, params: DataType[], values: unknown[]) {
    this.ensureOpen();
    return load({ library: LIBRARY_NAME, funcName: func, retType: ret, paramsType: params, paramsValue: values });
  }

  private callStatus(func: string, params: DataType[], values: unknown[] = []) {
    return this.call(func, DataType.I32, params, values) as number;
  }
}

let shared: TokenRouterFfiBinding | null = null;
export function getTokenRouterFfiBinding() {
  shared ??= new TokenRouterFfiBinding();
  return shared;
}
export function closeTokenRouterFfiBinding() {
  shared?.close();
  shared = null;
}
```

**C 函数 ↔ ffi-rs 映射**

| C 函数 | retType | paramsType |
|--------|---------|------------|
| `token_router_version` | `String` | `[]` |
| `token_router_start` | `I32` | `[String, U16, U8Array, U64]` |
| `token_router_stop` | `I32` | `[U8Array, U64]` |
| `token_router_is_running` | `I32` | `[]` |
| `token_router_gateway_url` | `I32` | `[U8Array, U64]` |

---

## 3. electron/utils/token-router-service.ts — 启动编排

FFI 只负责启动；上游配置走 Admin HTTP。**接入方主进程应调用 `ensureTokenRouterStarted()`**。

```ts
import { getTokenRouterFfiBinding } from './token-router-ffi';

const ADMIN_PORT = 11080; // 与 token_router_start 传入的 port 一致
const ADMIN = `http://127.0.0.1:${ADMIN_PORT}/v1/admin`;
const READY_TIMEOUT_MS = 15_000;
const POLL_MS = 300;

export interface TokenRouterStartOptions {
  homeDir: string;             // 数据目录，配置为 {homeDir}/config.toml
  port: number;                // 监听端口，如 11080
  cloudBaseUrl: string;        // 云端 API 根，须含 /v1，如 https://api.deepseek.com/v1
  cloudModel: string;          // UI 展示用；setup 时 cloud.model 固定写 'auto'
  cloudApiKey?: string;
  localModels: Array<{
    id: string;                // 端侧模型 id，如 qwen3:8b
    endpoint: string;          // 端侧 API 根，如 http://127.0.0.1:11434/v1
    contextWindow?: number;    // → gateway.ctx_edge_max_tokens
  }>;
}

export interface TokenRouterEndpoint {
  url: string;                 // http://127.0.0.1:11080/v1 — 给 Agent 的 baseUrl
  alreadyRunning: boolean;
  version?: string;
}

async function adminFetch<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${ADMIN}${path}`, init);
  if (!res.ok) throw new Error(`admin ${path} HTTP ${res.status}`);
  return res.json() as Promise<T>;
}

async function waitForReady() {
  const deadline = Date.now() + READY_TIMEOUT_MS;
  let lastErr: unknown;
  while (Date.now() < deadline) {
    try {
      const s = await adminFetch<{ listen?: string; version?: string }>('/status');
      if (s.listen) return s;
    } catch (e) {
      lastErr = e;
    }
    await new Promise((r) => setTimeout(r, POLL_MS));
  }
  throw new Error(`Router admin timeout: ${String(lastErr)}`);
}

function listenToBaseUrl(listen: string): string {
  const base = /^https?:\/\//i.test(listen) ? listen : `http://${listen}`;
  return base.replace(/\/+$/, '') + '/v1';
}

export async function ensureTokenRouterStarted(
  options: TokenRouterStartOptions,
): Promise<TokenRouterEndpoint> {
  const binding = getTokenRouterFfiBinding();
  const wasRunning = binding.isRunning();

  // 1. FFI 启动（home 与 port 必填）
  if (!wasRunning) {
    if (!options.homeDir || !options.port) {
      throw new Error('homeDir and port are required');
    }
    binding.start(options.homeDir, options.port);
  }

  // 2. 等 Admin HTTP 就绪
  const status = await waitForReady();

  // 3. 注入上游（每次调用都会热更新）
  const edge = options.localModels[0];
  await adminFetch('/setup', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      gateway: {
        route: 'auto',
        routing_mode: 'cascade',
        default_profile: 'balanced',
        ctx_edge_max_tokens: edge?.contextWindow,
      },
      cloud: {
        base_url: options.cloudBaseUrl,
        model: 'auto',
        ...(options.cloudApiKey ? { api_key: options.cloudApiKey } : {}),
      },
      ...(edge ? { edge: { base_url: edge.endpoint, model: edge.id } } : {}),
    }),
  });

  // 4. 返回 Agent 用的 baseUrl
  return {
    url: listenToBaseUrl(status.listen!),
    alreadyRunning: wasRunning,
    version: status.version,
  };
}

export async function getTokenRouterStatsSnapshot(scope: 'session' | 'global' = 'session') {
  return adminFetch(`/stats?scope=${scope}`);
}
```

**实现要点**

| 行为 | 说明 |
|------|------|
| 每次 `ensureStarted` 都 POST setup | 用户切换云端/端侧模型时会再次 invoke，须热更新而非仅首次写入 |
| 已运行时跳过 FFI start | `isRunning()` 为真时直接 `GET /status` + POST setup |
| `ADMIN` 端口须与 `port` 一致 | 建议 `const ADMIN = \`http://127.0.0.1:${options.port}/v1/admin\``，不要写死 |
| `cloudModel` 不入 setup | UI 展示字段；Gateway 侧 `cloud.model` 固定 `'auto'`（见 §3.1） |
| 仅用 `localModels[0]` | 多选时只取第一个作为 `[upstream.edge]` |
| Agent 不直连云/端 | 返回的 `url` 写入 OpenClaw `baseUrl`；模型 id 用 `Auto` / `auto` |

推荐把 setup 请求体抽成独立函数，便于单测与复用：

```ts
export function buildSetupBody(options: TokenRouterStartOptions) {
  const edge = options.localModels[0];
  return {
    gateway: {
      route: 'auto',
      routing_mode: 'cascade',
      default_profile: 'balanced',
      ...(edge?.contextWindow ? { ctx_edge_max_tokens: edge.contextWindow } : {}),
    },
    cloud: {
      base_url: options.cloudBaseUrl,
      model: 'auto',
      ...(options.cloudApiKey ? { api_key: options.cloudApiKey } : {}),
    },
    ...(edge ? { edge: { base_url: edge.endpoint, model: edge.id } } : {}),
  };
}
```

若 `config.toml` 配置了 `gateway.admin_token`，在 `adminFetch` 中加请求头：

```ts
const headers: Record<string, string> = { 'content-type': 'application/json' };
if (process.env.TOKEN_ROUTER_ADMIN_TOKEN) {
  headers['x-token-router-admin-token'] = process.env.TOKEN_ROUTER_ADMIN_TOKEN;
}
```

---

## 3.1 端云上游配置与 POST /v1/admin/setup

FFI 只负责把 Gateway 线程拉起来；**端侧模型、云端模型、路由策略** 全部通过 Admin HTTP 写入。FlowyClaw 在 `ensureFlowyRouterStarted()` 里自动 POST；其他宿主也可手动 curl 或调同一接口。

### 配置写入流程

```
ModelSelect（渲染进程）
  cloudBaseUrl / cloudApiKey / localModels[]
    ↓ invokeIpc('token-router:ensureStarted', options)
主进程 ensureTokenRouterStarted()
    ↓ FFI start（若未运行）
    ↓ GET /v1/admin/status（轮询就绪）
    ↓ POST /v1/admin/setup  ← 本文重点
    ↓ 写回 {homeDir}/config.toml + 内存热重载
返回 { url: 'http://127.0.0.1:11080/v1' }
    ↓
写入 OpenClaw provider.baseUrl；Agent 请求 model=Auto
```

### FlowyClaw 入参 → setup 字段映射

| 渲染进程 / IPC 字段 | setup JSON 路径 | 必填 | 说明 |
|---------------------|-----------------|------|------|
| `cloudBaseUrl` | `cloud.base_url` | 是 | OpenAI 兼容 API **根路径**，须含 `/v1`，如 `https://api.deepseek.com/v1` |
| `cloudApiKey` | `cloud.api_key` | 否 | Flowy 登录 token / 第三方 `sk-...`；省略则保留 config 中已有 key |
| `cloudModel` | — | — | **不写入 setup**；仅 UI 标签。Gateway 固定 `cloud.model: "auto"` |
| `localModels[0].endpoint` | `edge.base_url` | 否* | 端侧 API 根，如 `http://127.0.0.1:11434/v1`（Ollama）或 Herdsman HTTP |
| `localModels[0].id` | `edge.model` | 否* | 端侧模型名，如 `qwen3:8b` |
| `localModels[0].contextWindow` | `gateway.ctx_edge_max_tokens` | 否 | 端侧上下文 token 上限；超过约 80% 触发升云 |
| — | `gateway.route` | — | FlowyClaw 固定 `"auto"` |
| — | `gateway.routing_mode` | — | FlowyClaw 固定 `"cascade"`（先 edge，质量不足再升 cloud） |
| — | `gateway.default_profile` | — | FlowyClaw 固定 `"balanced"` |

\* 纯云端场景可省略 `edge` 块；纯端侧须配 `edge` 且 `route` 可改为 `"edge"`（FlowyClaw 默认混合路由故保留 `auto`）。

### 等价 POST 示例（FlowyClaw 一次 ensureStarted）

```bash
curl -s http://127.0.0.1:11080/v1/admin/setup \
  -H "Content-Type: application/json" \
  -d '{
    "gateway": {
      "route": "auto",
      "routing_mode": "cascade",
      "default_profile": "balanced",
      "ctx_edge_max_tokens": 65536
    },
    "cloud": {
      "base_url": "https://api.deepseek.com/v1",
      "model": "auto",
      "api_key": "sk-your-key"
    },
    "edge": {
      "base_url": "http://127.0.0.1:11434/v1",
      "model": "qwen3:8b"
    }
  }' | jq .
```

成功响应：

```json
{
  "ok": true,
  "message": "setup updated",
  "upstream": {
    "gateway": { "route": "auto", "routing_mode": "cascade", "ctx_edge_max_tokens": 65536, "...": "..." },
    "edge": {
      "configured": true,
      "base_url": "http://127.0.0.1:11434/v1",
      "model": "qwen3:8b",
      "api_key_set": false
    },
    "cloud": {
      "configured": true,
      "base_url": "https://api.deepseek.com/v1",
      "model": "auto",
      "api_key_set": true
    }
  }
}
```

写入后 `{homeDir}/config.toml` 对应段落：

```toml
[gateway]
route = "auto"
routing_mode = "cascade"
default_profile = "balanced"
ctx_edge_max_tokens = 65536

[upstream.edge]
base_url = "http://127.0.0.1:11434/v1"
model = "qwen3:8b"

[upstream.cloud]
base_url = "https://api.deepseek.com/v1"
model = "auto"
api_key = "sk-your-key"
```

### 请求体结构（`UpstreamSetupUpdate`）

定义见 `src/config/setup.rs`。所有字段均为 **partial patch**——省略的键保持 `config.toml` 原值不变。

```jsonc
{
  "agent_id": null,           // 可选；见下文「Agent 专属配置」
  "gateway": {                // 可选；路由 / 经验 / 自适应等
    "route": "auto",          // auto | edge | cloud | cascade
    "routing_mode": "cascade",// single | cascade | split（仅 route=auto）
    "default_profile": "balanced", // economy | balanced | premium | privacy
    "ctx_edge_max_tokens": 65536,
    "experience_enabled": true,
    "work_verify_sample_rate": 0.1,
    "adaptive_routing_enabled": true
    // ... 更多 gateway 字段见 README §6.1
  },
  "cloud": {
    "base_url": "https://api.deepseek.com/v1",
    "model": "auto",          // 推荐 auto；或固定如 deepseek-chat
    "api_key": "sk-...",
    "clear": false,           // true → 清除整个 cloud tier
    "token_budget": 500000    // 仅配合 agent_id 使用
  },
  "edge": {
    "base_url": "http://127.0.0.1:11434/v1",
    "model": "qwen3:8b",
    "api_key": "",            // 空字符串清除 key
    "clear": false            // true → 删除 [upstream.edge]
  }
}
```

#### `cloud.model = "auto"` 的含义

- 升云时**保留 Agent 请求里的 model 字段**，由 Token Router 根据难度与路由策略选择是否升云、升哪条路径。
- FlowyClaw 故意不把 UI 选的 `cloudModel.id` 写入 setup，避免 Agent 侧 model 与 Router 路由逻辑冲突。
- Agent 侧应使用单一入口模型（如 `token-router/Auto`），不要直接把 `deepseek-chat` 当作 primary。

#### `edge` / `cloud` patch 语义

| 操作 | JSON | 效果 |
|------|------|------|
| 设置 URL + 模型 | `{ "base_url": "...", "model": "..." }` | 创建或更新该 tier |
| 只改 API Key | `{ "api_key": "sk-..." }` | 其他字段不变 |
| 清除 API Key | `{ "api_key": "" }` | 置空 key |
| 移除端侧 | `{ "edge": { "clear": true } }` | 删除 `[upstream.edge]` |
| 移除云端 | `{ "cloud": { "clear": true } }` | 删除 `[upstream.cloud]` |

### GET 读取当前配置

```bash
# 全局
curl -s http://127.0.0.1:11080/v1/admin/setup | jq .

# 某 Agent 专属（OpenClaw / Hermes 等）
curl -s 'http://127.0.0.1:11080/v1/admin/setup?agent_id=hermes' | jq .
```

`edge` / `cloud` 视图字段：

| 字段 | 说明 |
|------|------|
| `configured` | `base_url` 非空即为 true |
| `base_url` | 上游根 URL |
| `model` | 配置的模型 id；cloud 默认可见 `"auto"` |
| `api_key_set` | 是否已配置 key（不返回明文） |
| `token_budget` | Agent 专属云 token 预算（若有） |

### Agent 专属配置（`agent_id`）

宿主可为不同 Agent 配置独立上游或云 token 预算（桌面版 Agents 页同款能力）：

```bash
curl -s http://127.0.0.1:11080/v1/admin/setup \
  -H "Content-Type: application/json" \
  -d '{
    "agent_id": "hermes",
    "cloud": {
      "base_url": "https://api.anthropic.com/v1",
      "api_key": "sk-ant-xxx",
      "model": "claude-sonnet-4-20250514",
      "token_budget": 500000
    },
    "edge": {
      "base_url": "http://127.0.0.1:11435/v1",
      "model": "qwen3:8b"
    }
  }' | jq .
```

FlowyClaw 默认**不写 `agent_id`**，即修改全局 `[upstream]`。若需 per-agent 路由，在 POST 体中加 `agent_id` 并在 GET 时用同名 query。

### 恢复默认

```bash
curl -s -X POST http://127.0.0.1:11080/v1/admin/setup/init | jq .
```

效果：cloud model=`auto`、cloud base_url 空、edge 清除。等同 CLI `token-router setup --init`。

### Web 配置页

浏览器打开 `http://127.0.0.1:<port>/setup`（与 `gateway.listen` 同 host/port）。页面表单提交的 JSON 与 `POST /v1/admin/setup` 相同，便于手工调试。

### 鉴权

| 配置项 | 保护范围 | 请求头 |
|--------|----------|--------|
| `gateway.admin_token` | POST setup / setup/init / shutdown / restart | `X-Token-Router-Admin-Token: <token>` |
| `gateway.api_key` | 入站 Chat API（`/v1/chat/completions` 等） | `Authorization: Bearer <key>` 或 `x-api-key` |

### 常见配置错误

| 现象 | 原因 | 修复 |
|------|------|------|
| 云端 404 | `base_url` 缺少 `/v1` | 改为 `https://api.xxx.com/v1` |
| 端侧连不上 | Ollama/Herdsman 未启动或端口错 | 确认 `edge.base_url` 可 `curl` |
| setup 返回 401 | 配置了 `admin_token` 但未带头 | 加 `X-Token-Router-Admin-Token` |
| Agent 仍走旧模型 | 未重新 `ensureStarted` | 切换模型后须再次 POST setup |
| 多本地模型无效 | 仅 `localModels[0]` 生效 | 改 service 层逻辑或只传一个 |

更完整的 setup curl 合集见 [README.md §5.1](./README.md#51-setup-api-调用示例)。

#### FlowyClaw 默认路由参数含义

| 字段 | 值 | 行为摘要 |
|------|-----|----------|
| `gateway.route` | `auto` | 按请求难度 + profile 自动选 edge/cloud/cascade |
| `gateway.routing_mode` | `cascade` | 中等难度：**先走端侧**，质量校验不过再升云 |
| `gateway.default_profile` | `balanced` | 难度阈值居中；要更多端侧可改为 `economy` |

若需**强制全云端**（调试）：`POST { "gateway": { "route": "cloud" } }`。  
若需**强制全端侧**：`POST { "gateway": { "route": "edge" } }` 且必须配置 `edge`。

---

## 4. 主进程 IPC Handler

在 `electron/main/ipc-handlers.ts`（或独立 flowy 模块）注册。**推荐在主进程补全 `homeDir` / `port`**，避免渲染进程接触文件系统路径：

```ts
import { app, ipcMain } from 'electron';
import { join } from 'node:path';
import {
  ensureTokenRouterStarted,
  getTokenRouterStatsSnapshot,
  type TokenRouterStartOptions,
} from '../utils/token-router-service';

const ROUTER_HOME = join(app.getPath('userData'), 'token-router');
const ROUTER_PORT = 11080;

export function registerTokenRouterIpc() {
  ipcMain.handle(
    'token-router:ensureStarted',
    async (_evt, options: Omit<TokenRouterStartOptions, 'homeDir' | 'port'> & Partial<Pick<TokenRouterStartOptions, 'homeDir' | 'port'>>) => {
      return await ensureTokenRouterStarted({
        homeDir: options.homeDir ?? ROUTER_HOME,
        port: options.port ?? ROUTER_PORT,
        cloudBaseUrl: options.cloudBaseUrl,
        cloudModel: options.cloudModel,
        cloudApiKey: options.cloudApiKey,
        localModels: options.localModels,
      });
    },
  );

  ipcMain.handle(
    'token-router:getStats',
    async (_evt, scope?: 'session' | 'global') => {
      return await getTokenRouterStatsSnapshot(scope ?? 'session');
    },
  );
}

// app.whenReady() 中调用:
// registerTokenRouterIpc();
```

| Channel | 入参 | 返回值 | 作用 |
|---------|------|--------|------|
| `token-router:ensureStarted` | `TokenRouterStartOptions`（`homeDir`/`port` 可省略） | `{ url, alreadyRunning, version? }` | FFI 启动 + POST setup + 返回 Agent baseUrl |
| `token-router:getStats` | `scope?: 'session' \| 'global'` | 统计 JSON | 路由分布、Token、延迟等 |

**数据目录建议**

| 宿主 | 推荐 `homeDir` |
|------|----------------|
| FlowyClaw 类应用 | `join(app.getPath('userData'), 'token-router')` |
| 与 CLI 共用配置 | `join(app.getPath('home'), '.token-router')` |
| 开发隔离 | `/tmp/token-router-dev` 或 `%TEMP%\token-router-dev` |

目录结构（首次启动后自动创建）：

```
{homeDir}/
  config.toml       # setup POST 写回的目标
  stats.json        # 全局累计统计
  experience.json   # 路由经验
  sessions/         # 会话粘性等
  logs/gateway.log  # 排错首选
```

---

## 5. preload 白名单 + 暴露 API

**electron/flowy/ipc-channels.ts**

```ts
export const TOKEN_INVOKE_CHANNELS = [
  'token-router:ensureStarted',
  'token-router:getStats',
  // ...其他 channel
] as const;
```

**electron/preload/index.ts**（按项目现有模式）

```ts
import { contextBridge, ipcRenderer } from 'electron';

contextBridge.exposeInMainWorld('electron', {
  invoke: (channel: string, ...args: unknown[]) =>
    ipcRenderer.invoke(channel, ...args),
});
```

渲染进程通过项目的 `invokeIpc` 封装调用，例如 `invokeIpc('token-router:ensureStarted', options)`。

---

## 6. 渲染进程调用 + OpenClaw Provider

**src/components/ModelSelect.tsx**（混合路由：云端 + 本地）

```ts
import { invokeIpc } from '@/lib/api-client';

async function startHybridRouter() {
  // homeDir / port 可由主进程 IPC 层补全（见 §4）
  const router = await invokeIpc<{
    url: string;
    alreadyRunning: boolean;
    version?: string;
  }>('token-router:ensureStarted', {
    cloudBaseUrl: cloudModel.endpoint,   // 须含 /v1，如 Flowy API 根
    cloudModel: cloudModel.id,           // UI 展示；不写入 Gateway
    cloudApiKey: authToken,              // Flowy 登录 JWT / 云端 sk-...
    localModels: localModels.map((m) => ({
      id: m.id,                          // → edge.model
      endpoint: m.endpoint,              // → edge.base_url
      contextWindow: m.contextWindow,    // → gateway.ctx_edge_max_tokens
    })),
  });

  if (!router?.url) throw new Error('Token Router did not return gateway URL');

  await writeOpenClawProvider(router.url);
  return router.url;
}
```

### UI 模型列表 → setup 字段（FlowyClaw 惯例）

| UI 数据源 | 典型值 | setup 去向 |
|-----------|--------|------------|
| 云端模型 `endpoint` | `https://api.flowy.ai/v1` 或 DeepSeek 等 | `cloud.base_url` |
| 登录态 `authToken` | Bearer / JWT | `cloud.api_key` |
| 云端模型 `id` | `gpt-4` 等 | **不入 setup**（仅列表展示） |
| 本地模型 `endpoint` | `http://127.0.0.1:11434/v1` | `edge.base_url`（仅 `[0]`） |
| 本地模型 `id` | `qwen3:8b` | `edge.model` |
| 本地模型 `contextWindow` | `32768` | `gateway.ctx_edge_max_tokens` |

用户**每次切换模型并确认**后应重新调用 `ensureStarted`，触发新的 `POST /v1/admin/setup`（见 §3.1）。

### OpenClaw Provider 写入

**~/.openclaw/openclaw.json** — 启动后写入的 Provider 结构：

```json
{
  "models": {
    "providers": {
      "token-router": {
        "baseUrl": "http://127.0.0.1:11080/v1",
        "apiKey": "",
        "models": [
          { "id": "Auto", "name": "Token Router Auto" }
        ]
      }
    }
  },
  "agents": {
    "defaults": {
      "model": {
        "primary": "token-router/Auto"
      }
    }
  }
}
```

Agent（OpenClaw / Hermes / OpenCode）只需把 `baseUrl` 指向 Router；路由决策由 Gateway 完成，Agent 代码无需改动。

### Agent 请求路径

```
OpenClaw Agent
  POST http://127.0.0.1:11080/v1/chat/completions
  { "model": "auto", "messages": [...] }
    ↓ Token Router 读 config.toml [upstream.edge/cloud]
    ↓ 按 route=routing_mode=default_profile 决策
    → 端侧 http://127.0.0.1:11434/v1  或  云端 https://api.deepseek.com/v1
```

其他兼容端点（桌面版 Status Pipe 也会暴露）：

| 路径 | 用途 |
|------|------|
| `/v1/chat/completions` | OpenAI Chat（主路径） |
| `/v1/responses` | OpenAI Responses API |
| `/anthropic/v1/messages` | Anthropic 兼容 |

---

## 7. vite.config + electron-builder 打包

**vite.config.ts** — 主进程 rollup external：

```ts
rollupOptions: {
  external: ['ffi-rs', /\.node$/],
},
```

**electron-builder.yml**

```yaml
extraResources:
  - from: resources/
    to: resources/
    filter:
      - "**/*"

# 打包后 DLL 路径:
# process.resourcesPath/resources/win32/x64/token_router.dll
```

**CI 脚本示例**

```bash
# 1. 构建 DLL
cd token-router && make release-dylib
cp target/release/token_router.dll ../your-app/resources/win32/x64/

# 2. 打包 Electron
cd ../your-app && pnpm install && pnpm run package:win
```

---

## 8. 应用退出清理

**electron/main/index.ts**

```ts
import { getTokenRouterFfiBinding, closeTokenRouterFfiBinding } from '../utils/token-router-ffi';

app.on('before-quit', () => {
  try {
    if (getTokenRouterFfiBinding().isRunning()) {
      getTokenRouterFfiBinding().stop();
    }
  } finally {
    closeTokenRouterFfiBinding();
  }
});
```

---

## 9. koffi 冒烟测试（可选）

不依赖 Electron，验证 DLL 能否加载。token-router 仓库 `example/electron/main.mjs`：

```js
import koffi from "koffi";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const base = path.join(__dirname, "../../target/release");

function libraryPath() {
  if (process.platform === "win32") return path.join(base, "token_router.dll");
  if (process.platform === "darwin") return path.join(base, "libtoken_router.dylib");
  return path.join(base, "libtoken_router.so");
}

const lib = koffi.load(libraryPath());
const TOKEN_OK = 0;

const token_router_version = lib.func("const char *token_router_version()");
const token_router_start = lib.func(
  "int32 token_router_start(const char *home_dir, uint16 port, _Out_ char *error_out, size_t error_out_len)",
);
const token_router_stop = lib.func(
  "int32 token_router_stop(_Out_ char *error_out, size_t error_out_len)",
);
const token_router_is_running = lib.func("int32 token_router_is_running()");
const token_router_gateway_url = lib.func(
  "int32 token_router_gateway_url(_Out_ char *url_out, size_t url_out_len)",
);

const homeDir = process.argv[2];
const port = Number(process.argv[3]);
if (!homeDir || !Number.isInteger(port) || port <= 0) {
  console.error("usage: node main.mjs <home_dir> <port>");
  process.exit(1);
}

const errorBuf = Buffer.alloc(4096);
const urlBuf = Buffer.alloc(256);

console.log("version:", token_router_version());
if (token_router_start(homeDir, port, errorBuf, errorBuf.length) !== TOKEN_OK) {
  console.error("start failed:", errorBuf.toString("utf8").replace(/\0.*$/, ""));
  process.exit(1);
}
token_router_gateway_url(urlBuf, urlBuf.length);
console.log("url:", urlBuf.toString("utf8").replace(/\0.*$/, ""));
console.log("running:", Boolean(token_router_is_running()));
token_router_stop(errorBuf, errorBuf.length);
```

```bash
cd token-router/example/electron && npm install
node main.mjs /tmp/token-router-dev 11080
```

生产环境推荐 ffi-rs；koffi 适合快速验证。

---

## 10. 验证命令

Gateway 启动后（以下示例端口 `11080`，按实际 `port` 替换）：

```bash
# 健康检查
curl -s http://127.0.0.1:11080/health

# 查看状态（listen 地址、版本）
curl -s http://127.0.0.1:11080/v1/admin/status | jq .

# 读取当前上游（确认 setup 已写入）
curl -s http://127.0.0.1:11080/v1/admin/setup | jq '.edge, .cloud, .gateway.route'

# 手动 setup — 等同 FlowyClaw ensureStarted 内的 POST
curl -s http://127.0.0.1:11080/v1/admin/setup \
  -H "Content-Type: application/json" \
  -d '{
    "gateway": {
      "route": "auto",
      "routing_mode": "cascade",
      "default_profile": "balanced",
      "ctx_edge_max_tokens": 65536
    },
    "cloud": {
      "base_url": "https://api.deepseek.com/v1",
      "model": "auto",
      "api_key": "sk-..."
    },
    "edge": {
      "base_url": "http://127.0.0.1:11434/v1",
      "model": "qwen3:8b"
    }
  }' | jq .

# 仅云端
curl -s http://127.0.0.1:11080/v1/admin/setup \
  -H "Content-Type: application/json" \
  -d '{"cloud":{"base_url":"https://api.deepseek.com/v1","model":"auto","api_key":"sk-..."}}' | jq .

# 清除端侧
curl -s http://127.0.0.1:11080/v1/admin/setup \
  -H "Content-Type: application/json" \
  -d '{"edge":{"clear":true}}' | jq .

# 路由 smoke（经 Router 转发）
curl -s http://127.0.0.1:11080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"auto","messages":[{"role":"user","content":"hi"}]}'

# 会话统计（等同 token-router:getStats scope=session）
curl -s 'http://127.0.0.1:11080/v1/admin/stats?scope=session' | jq .

# 路由决策日志
curl -s 'http://127.0.0.1:11080/v1/admin/routing-logs?limit=5' | jq .
```

**端到端自检顺序**

1. `curl /health` → `ok`
2. `curl /v1/admin/setup` → `edge.configured` / `cloud.configured` 符合预期
3. `curl /v1/chat/completions` → 有响应；查 `routing-logs` 可见 `route` / `served_route`
4. 修改 setup 后**无需重启** Gateway，直接重复步骤 2–3

---

## 11. Tauri / Rust crate 嵌入（可选）

若宿主是 **Tauri** 或其他 Rust 应用，可直接依赖 `token-router` crate，调用 `embedded` 模块，**无需 DLL / ffi-rs**。

**desktop/src-tauri/Cargo.toml**

```toml
[dependencies]
token-router = { path = "../../", features = ["desktop"] }
```

**启动 Gateway（Rust）**

```rust
use token_router::embedded;

// home=None, port=None → 使用默认 home（~/.token-router/）与 config.toml 中的端口
let url = embedded::start(None, None)?;

// 或显式指定（等同 FFI）
let url = embedded::start(Some(home_dir.as_ref()), Some(11080))?;

// 停止
embedded::stop()?;
```

**暴露给前端的 Tauri 命令**（见 `desktop/src-tauri/src/lib.rs`）：

| 命令 | 说明 |
|------|------|
| `gateway_start` | 启动 embedded Gateway |
| `gateway_stop` | 停止 |
| `gateway_restart` | 重启 |
| `gateway_is_running` | 是否运行中 |
| `gateway_url` | `http://host:port` |
| `gateway_status` | `{ running, url, version }` |
| `gateway_read_logs` | 读取 `gateway.log` |
| `gateway_read_routing_logs` | 路由决策日志 |

上游配置仍通过 Admin HTTP（`POST /v1/admin/setup`）或 Web 配置页 `/setup` 完成，与 Electron 路径一致。

桌面版数据目录：`~/.token-router-desktop/`（与 CLI 默认 `~/.token-router/` 分离）。详见 [`desktop/README.md`](./desktop/README.md)。

---

## 12. Gateway 发现（Status Pipe）

Token Router 桌面版向第三方客户端暴露 Gateway 地址（协议与 [Herdsman](https://github.com/szStarWave/herdsman) 对称）。

| 平台 | 通道 | 命令 |
|------|------|------|
| Windows | 命名管道 `\\.\pipe\Token-Router-status` | `/status` → JSON |
| macOS / Linux | Unix socket `~/.token-router-desktop/Token-Router-status.sock` | `/status` → JSON |

`/status` 响应含 `endpoint`、`openai_endpoint`、`chat_endpoint`、`responses_endpoint`、`anthropic_endpoint`、`webui_url` 等字段。PowerShell / `nc` 示例见 [`desktop/README.md`](./desktop/README.md#ipc-statusgateway-发现)。

Electron 宿主若自行嵌入 DLL，可按需实现类似发现机制，或让 Agent 直接读取 `ensureTokenRouterStarted()` 返回的 `url`。

---

## 接入检查清单

| # | 动作 | 验证 |
|---|------|------|
| 1 | `make release-dylib` → 拷贝到 `resources/win32/x64/` | 文件存在 |
| 2 | 安装 `ffi-rs`，vite external | 主进程 build 无 bundle 报错 |
| 3 | 创建 token-router-ffi.ts + token-router-service.ts | `node main.mjs <home> <port>` 通过 |
| 4 | 注册 IPC handler + preload 白名单 | 渲染进程 invoke 不报错 |
| 5 | UI 调用 ensureStarted，写 OpenClaw provider | `curl /v1/admin/setup` 中 edge/cloud 正确 |
| 5b | 切换模型后再次 ensureStarted | setup 热更新，`routing-logs` 反映新上游 |
| 6 | electron-builder extraResources | 打包后 DLL 在 resourcesPath |
| 7 | before-quit 调 stop | 退出无僵尸线程 |

---

## 与独立 CLI / Tauri 模式对比

| 维度 | DLL 嵌入（Electron） | Rust crate（Tauri） | CLI 守护进程 |
|------|---------------------|---------------------|--------------|
| 进程模型 | Gateway 在 Electron 主进程内 | Gateway 在 Tauri 主进程内 | 独立 `token-router gateway start` |
| 部署 | 随安装包分发 DLL | 静态链接 / 同 crate | 需单独安装或 PATH |
| 配置 | Admin HTTP 热更新 + config.toml | 同上 | config.toml + CLI setup |
| 跨平台 | 需各平台 dylib/so | Windows / macOS / Linux 均已支持 | 全平台 |
| 适用场景 | Electron 一体化产品 | Tauri 桌面壳 | 服务器、开发机 |

三种模式共享同一套 Gateway 逻辑与 HTTP 接口；仅启动入口不同。

---

## 参考文件索引

### Token Router（本仓库）

| 路径 | 说明 |
|------|------|
| `src/ffi.rs` | C ABI 实现 |
| `src/embedded.rs` | 进程内 Gateway 线程 |
| `ffi/token_router.h` | C 头文件 |
| `example/electron/main.mjs` | koffi 最小示例 |
| `Makefile` | `release-dylib`、`package-electron-win`、`tauri-build` 等 |
| `packaging/electron-win32/` | `package-electron-win` 分发包模板与示例 |
| `desktop/` | Tauri 桌面壳（Rust `embedded` 参考实现） |
| `src/config/setup.rs` | `UpstreamSetupUpdate` 结构与 patch 逻辑 |
| `src/gateway/api/setup.rs` | `/v1/admin/setup` HTTP handler + Web `/setup` 页 |
| `example/config.toml` | 完整 TOML 模板（edge + cloud + gateway） |
| `desktop/frontend/src/lib/cloud-upstream.ts` | 桌面版云端 POST setup |
| `desktop/frontend/src/lib/edge-upstream.ts` | 桌面版端侧 POST setup |
| `packaging/electron-win32/` | Windows Electron 分发包模板 |
| `incept.html` | 本文 HTML 版 |

### FlowyClaw（Electron 参考宿主）

> 文件名仍使用 `flowy-router-*` 前缀，逻辑与本文 `token-router-*` 示例一致。

| 路径 | 说明 |
|------|------|
| `electron/utils/flowy-router-ffi.ts` | ffi-rs 绑定（完整版） |
| `electron/utils/flowy-router-service.ts` | 启动编排 + Admin API |
| `electron/flowy/ipc-handlers.ts` | IPC handler |
| `electron/flowy/ipc-channels.ts` | preload 白名单 |
| `src/components/layout/ModelSelect.tsx` | 渲染进程调用示例 |
| `vite.config.ts` | ffi-rs external 配置 |
| `electron-builder.yml` | extraResources 打包 |

---

## 常见问题

**DLL not found** — 检查 `resources/{platform}/{arch}/` 是否存在；开发模式下 cwd 是否为项目根目录。

**Timed out waiting for admin HTTP** — FFI start 成功但 Gateway 未监听：查看 `{homeDir}/logs/gateway.log`；确认 `port` 未被占用；`ADMIN` 常量须与 `token_router_start` 传入端口一致。

**AlreadyRunning** — 同进程重复 start；先 `isRunning()`，已运行则跳过 start，直接走 status/setup。

**跨平台 DLL** — `token-router-ffi.ts` 已含 win32/darwin/linux 路径解析；各平台需对应 `release-dylib` 产物。Tauri 路径无需 DLL，见 [§11](#11-tauri--rust-crate-嵌入可选)。

**配置不生效** — 确认 `POST /v1/admin/setup` 返回 `ok: true`；检查 `{homeDir}/config.toml` 是否更新；若配置了 `gateway.admin_token`，请求须带 `X-Token-Router-Admin-Token`。

**云端 404 / 端侧 connection refused** — `base_url` 须含 `/v1`；端侧服务（Ollama/Herdsman）须先启动。用 `curl -s $edge_base_url/models` 单独验证。

**UI 选了云端模型但 Router 仍用 auto** — 符合设计：`cloud.model` 固定 `auto`，由 Router 决定升云时机；Agent primary 应为 `token-router/Auto`。

**多个本地模型只有第一个生效** — `ensureTokenRouterStarted` 仅取 `localModels[0]`；需多模型时扩展 service 层或让用户单选。

**切换模型后 Agent 行为不变** — 须重新 `invokeIpc('token-router:ensureStarted', ...)` 触发 setup；仅改 OpenClaw json 不够。

**与 Token Router 桌面版差异** — 桌面版在 UI 内分别 POST `{cloud}` / `{edge}`（见 `desktop/frontend/src/lib/cloud-upstream.ts`、`edge-upstream.ts`）；FlowyClaw 在每次 ensureStarted 时一次性 POST gateway+cloud+edge。
