# Token Router — Windows Electron 接入包

由 `make package-electron-win` 生成。包含 Windows x64 DLL、C 头文件、接入文档与示例代码。

## 目录结构

```
token-router-electron-win32-x64/
├── README.md                      ← 本文件
├── VERSION                        ← token-router crate 版本
├── incept.md                      ← 完整接入指南（Markdown）
├── incept.html                    ← 接入指南（HTML）
├── electron-builder.example.yml   ← electron-builder extraResources 示例
├── ffi/
│   └── token_router.h             ← C ABI 头文件
├── resources/
│   └── win32/x64/
│       └── token_router.dll       ← 拷贝到宿主项目的 resources/
├── bin/                           ← 可选：CLI 二进制（便于本地调试）
│   └── token-router.exe
├── config/
│   ├── config.toml                ← 推荐示例（edge + cloud）
│   ├── config.edge-only.toml
│   └── config.minimal.toml
├── docs/
│   ├── incept.md
│   └── incept.html
└── example/
    ├── README.md                  ← 配置示例说明
    ├── package.ffi-rs.json        ← 宿主推荐 npm 依赖（ffi-rs）
    ├── electron/                  ← koffi 完整示例（加载包内 DLL）
    │   ├── main.mjs
    │   └── package.json
    └── smoke/                     ← 冒烟测试
        ├── main.mjs
        └── package.json
```

## 快速接入

1. 将 `resources/win32/x64/token_router.dll` 复制到宿主 Electron 项目：

   ```text
   your-electron-app/resources/win32/x64/token_router.dll
   ```

2. 宿主 `package.json` 增加依赖（见 `example/package.ffi-rs.json`）。

3. 按 `incept.md`（或 `docs/incept.md`）实现 `token-router-ffi.ts`、`token-router-service.ts`、IPC 与 UI 调用。

4. `electron-builder.yml` 参考 `electron-builder.example.yml` 配置 `extraResources`。

5. 上游（端侧 / 云端）通过 `POST /v1/admin/setup` 热更新，详见 `incept.md` §3.1。

## 冒烟测试

### 完整示例（example/electron）

从包根目录运行（`main.mjs` 会向上查找 `resources/`）：

```powershell
cd example/electron
npm install
node main.mjs $env:TEMP\token-router-dev 11080
```

### 分发包 smoke（example/smoke）

```powershell
cd example/smoke
npm install
node main.mjs $env:TEMP\token-router-dev 11080
```

## 数据目录

FFI 启动时须传入 `home_dir` 与 `port`（无默认值）。配置写入 `{home_dir}/config.toml`。

CLI 默认 home：`%USERPROFILE%\.token-router\`。
