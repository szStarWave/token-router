# Flowy Router DLL 接入指南

> 面向 Electron 宿主（OpenCode / Codex / FlowyClaw 等）。按步骤创建下列文件即可接入：FFI 启动 Gateway → Admin HTTP 配置上游 → IPC 暴露给 UI → Agent 指向 `http://127.0.0.1:11080/v1`。
>
> 参考实现：FlowyClaw 项目 `token-router` 集成。HTML 版见 [incept.html](./incept.html)。

---

## 目录

0. [构建 DLL 并放入 resources](#0-构建-dll-并放入-resources)
1. [package.json 依赖](#1-packagejson-依赖)
2. [flowy-router-ffi.ts — FFI 绑定](#2-electronutilsflowy-router-ffits--ffi-绑定)
3. [flowy-router-service.ts — 启动编排](#3-electronutilsflowy-router-servicets--启动编排)
4. [ipc-handlers.ts — 主进程 IPC](#4-主进程-ipc-handler)
5. [ipc-channels.ts + preload — 白名单与暴露](#5-preload-白名单--暴露-api)
6. [渲染进程调用 + OpenClaw Provider](#6-渲染进程调用--openclaw-provider)
7. [vite.config + electron-builder 打包](#7-viteconfig--electron-builder-打包)
8. [应用退出清理](#8-应用退出清理)
9. [koffi 冒烟测试（可选）](#9-koffi-冒烟测试可选)
10. [验证命令](#10-验证命令)

---

## 目标文件树

```
your-electron-app/
├── resources/
│   └── win32/x64/token_router.dll      ← Step 0
├── electron/
│   ├── utils/
│   │   ├── flowy-router-ffi.ts         ← Step 2
│   │   └── flowy-router-service.ts     ← Step 3
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
  → 主进程 ensureFlowyRouterStarted()
  → FFI token_router_start('')
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
| **Chat HTTP** | Agent 入口（`/v1/chat/completions`） |

---

## 0. 构建 DLL 并放入 resources

在 FlowyRouter 仓库执行：

```bash
make release-dylib
# Windows 产物: target/release/token_router.dll

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
| `token_router_start(config_path, error_out, error_out_len)` | `int32` | 后台启动；`config_path` 可为 `NULL` 或空串 → 默认 `~/.token-router/config.toml` |
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

数据目录：Windows `%USERPROFILE%\.token-router\`，Unix `~/.token-router/`。

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

## 2. electron/utils/flowy-router-ffi.ts — FFI 绑定

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

export enum FlowyRouterStatusCode {
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

export class FlowyRouterFfiBinding {
  private opened = false;
  constructor(private readonly dllPath = defaultRouterDllPath()) {}

  start(configPath: string) {
    const errorOut = Buffer.alloc(ERROR_BUFFER_LEN);
    const code = this.callStatus(
      'token_router_start',
      [DataType.String, DataType.U8Array, DataType.U64],
      [configPath, errorOut, errorOut.length],
    );
    if (code !== FlowyRouterStatusCode.OK) {
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
    if (code !== FlowyRouterStatusCode.OK) {
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

let shared: FlowyRouterFfiBinding | null = null;
export function getFlowyRouterFfiBinding() {
  shared ??= new FlowyRouterFfiBinding();
  return shared;
}
export function closeFlowyRouterFfiBinding() {
  shared?.close();
  shared = null;
}
```

**C 函数 ↔ ffi-rs 映射**

| C 函数 | retType | paramsType |
|--------|---------|------------|
| `token_router_version` | `String` | `[]` |
| `token_router_start` | `I32` | `[String, U8Array, U64]` |
| `token_router_stop` | `I32` | `[U8Array, U64]` |
| `token_router_is_running` | `I32` | `[]` |
| `token_router_gateway_url` | `I32` | `[U8Array, U64]` |

---

## 3. electron/utils/flowy-router-service.ts — 启动编排

FFI 只负责启动；上游配置走 Admin HTTP。**接入方主进程应调用 `ensureFlowyRouterStarted()`**。

```ts
import { getFlowyRouterFfiBinding } from './flowy-router-ffi';

const ADMIN = 'http://127.0.0.1:11080/v1/admin';
const READY_TIMEOUT_MS = 15_000;
const POLL_MS = 300;

export interface FlowyRouterStartOptions {
  cloudBaseUrl: string;       // 云端 API 根，须含 /v1，如 https://api.deepseek.com/v1
  cloudModel: string;          // UI 展示用；setup 时 cloud.model 固定写 'auto'
  cloudApiKey?: string;
  localModels: Array<{
    id: string;                // 端侧模型 id，如 qwen3:8b
    endpoint: string;          // 端侧 API 根，如 http://127.0.0.1:11434/v1
    contextWindow?: number;     // → gateway.ctx_edge_max_tokens
  }>;
}

export interface FlowyRouterEndpoint {
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

export async function ensureFlowyRouterStarted(
  options: FlowyRouterStartOptions,
): Promise<FlowyRouterEndpoint> {
  const binding = getFlowyRouterFfiBinding();
  const wasRunning = binding.isRunning();

  // 1. FFI 启动（空字符串 = 默认 ~/.token-router/config.toml）
  if (!wasRunning) binding.start('');

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

export async function getFlowyRouterStatsSnapshot(scope: 'session' | 'global' = 'session') {
  return adminFetch(`/stats?scope=${scope}`);
}
```

---

## 4. 主进程 IPC Handler

在 `electron/main/ipc-handlers.ts`（或独立 flowy 模块）注册：

```ts
import { ipcMain } from 'electron';
import {
  ensureFlowyRouterStarted,
  getFlowyRouterStatsSnapshot,
  type FlowyRouterStartOptions,
} from '../utils/flowy-router-service';

export function registerTokenRouterIpc() {
  ipcMain.handle(
    'token-router:ensureStarted',
    async (_evt, options: FlowyRouterStartOptions) => {
      return await ensureFlowyRouterStarted(options);
    },
  );

  ipcMain.handle(
    'token-router:getStats',
    async (_evt, scope?: 'session' | 'global') => {
      return await getFlowyRouterStatsSnapshot(scope ?? 'session');
    },
  );
}

// app.whenReady() 中调用:
// registerTokenRouterIpc();
```

| Channel | 作用 |
|---------|------|
| `token-router:ensureStarted` | 启动 + 配置上游，返回 baseUrl |
| `token-router:getStats` | 路由统计（session / global） |

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
  const router = await invokeIpc<{
    url: string;
    alreadyRunning: boolean;
    version?: string;
  }>('token-router:ensureStarted', {
    cloudBaseUrl: cloudModel.endpoint,   // 须含 /v1
    cloudModel: cloudModel.id,
    cloudApiKey: authToken,              // 可选
    localModels: localModels.map((m) => ({
      id: m.id,
      endpoint: m.endpoint,              // 如 http://127.0.0.1:11434/v1
      contextWindow: m.contextWindow,
    })),
  });

  if (!router?.url) throw new Error('Flowy Router did not return gateway URL');

  // router.url === 'http://127.0.0.1:11080/v1'
  return router.url;
}
```

**~/.openclaw/openclaw.json** — 启动后写入的 Provider 结构：

```json
{
  "models": {
    "providers": {
      "token-router": {
        "baseUrl": "http://127.0.0.1:11080/v1",
        "apiKey": "",
        "models": [
          { "id": "Auto", "name": "Flowy Auto Route" }
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
import { getFlowyRouterFfiBinding, closeFlowyRouterFfiBinding } from '../utils/flowy-router-ffi';

app.on('before-quit', () => {
  try {
    if (getFlowyRouterFfiBinding().isRunning()) {
      getFlowyRouterFfiBinding().stop();
    }
  } finally {
    closeFlowyRouterFfiBinding();
  }
});
```

---

## 9. koffi 冒烟测试（可选）

不依赖 Electron，验证 DLL 能否加载。FlowyRouter 仓库 `example/electron/main.mjs`：

```js
import koffi from "koffi";
import path from "node:path";

const dll = path.join("../../target/release", "token_router.dll");
const lib = koffi.load(dll);

const TOKEN_OK = 0;
const start = lib.func(
  "int32 token_router_start(const char *config_path, _Out_ char *error_out, size_t error_out_len)",
);
const isRunning = lib.func("int32 token_router_is_running()");
const gatewayUrl = lib.func(
  "int32 token_router_gateway_url(_Out_ char *url_out, size_t url_out_len)",
);
const stop = lib.func(
  "int32 token_router_stop(_Out_ char *error_out, size_t error_out_len)",
);

const errBuf = Buffer.alloc(512);
const urlBuf = Buffer.alloc(256);

if (start(null, errBuf, errBuf.length) !== TOKEN_OK) process.exit(1);
console.log("running:", isRunning());
gatewayUrl(urlBuf, urlBuf.length);
console.log("url:", urlBuf.toString().split("\0")[0]);
stop(errBuf, errBuf.length);
```

```bash
cd token-router/example/electron && npm install && node main.mjs
```

生产环境推荐 ffi-rs；koffi 适合快速验证。

---

## 10. 验证命令

Gateway 启动后：

```bash
# 健康检查
curl -s http://127.0.0.1:11080/health

# 查看状态
curl -s http://127.0.0.1:11080/v1/admin/status | jq .

# 手动 setup（等同 service 层 POST）
curl -s http://127.0.0.1:11080/v1/admin/setup \
  -H "Content-Type: application/json" \
  -d '{
    "gateway": { "route": "auto", "routing_mode": "cascade" },
    "cloud": { "base_url": "https://api.deepseek.com/v1", "model": "auto", "api_key": "sk-..." },
    "edge":  { "base_url": "http://127.0.0.1:11434/v1", "model": "qwen3:8b" }
  }'

# 路由 smoke
curl -s http://127.0.0.1:11080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"flowy-auto","messages":[{"role":"user","content":"hi"}]}'
```

---

## 接入检查清单

| # | 动作 | 验证 |
|---|------|------|
| 1 | `make release-dylib` → 拷贝到 `resources/win32/x64/` | 文件存在 |
| 2 | 安装 `ffi-rs`，vite external | 主进程 build 无 bundle 报错 |
| 3 | 创建 ffi.ts + service.ts | `node example/electron/main.mjs` 通过 |
| 4 | 注册 IPC handler + preload 白名单 | 渲染进程 invoke 不报错 |
| 5 | UI 调用 ensureStarted，写 OpenClaw provider | `curl /health` OK |
| 6 | electron-builder extraResources | 打包后 DLL 在 resourcesPath |
| 7 | before-quit 调 stop | 退出无僵尸线程 |

---

## 与独立 CLI 模式对比

| 维度 | DLL 嵌入 | CLI 守护进程 |
|------|----------|--------------|
| 进程模型 | Gateway 在 Electron 主进程内 | 独立 `token-router gateway start` 子进程 |
| 部署 | 随安装包分发 DLL | 需单独安装 token-router 或 PATH |
| 配置 | Admin HTTP 热更新 + 可选 config.toml | config.toml + CLI setup |
| 适用场景 | 桌面一体化产品 | 服务器、开发机 |

两种模式共享同一套 Gateway 逻辑与 HTTP 接口；仅启动入口不同。

---

## 参考文件索引

### FlowyRouter（本仓库）

| 路径 | 说明 |
|------|------|
| `src/ffi.rs` | C ABI 实现 |
| `src/embedded.rs` | 进程内 Gateway 线程 |
| `ffi/token_router.h` | C 头文件 |
| `example/electron/main.mjs` | koffi 最小示例 |
| `Makefile` | `release-dylib` 目标 |
| `incept.html` | 本文 HTML 版 |

### FlowyClaw（参考宿主）

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

**Timed out waiting for admin HTTP** — FFI start 成功但 Gateway 未监听：查看 `~/.token-router/logs/gateway.log`；确认端口 11080 未被占用。

**AlreadyRunning** — 同进程重复 start；先 `isRunning()`，已运行则跳过 start，直接走 status/setup。

**仅 Windows 支持** — FlowyClaw 当前限制 win32/x64；macOS / Linux 需扩展路径解析并准备 dylib/so。

**配置不生效** — 确认 `POST /v1/admin/setup` 返回 `ok: true`；若配置了 `gateway.admin_token`，请求须带 `X-Token-Router-Admin-Token`。
