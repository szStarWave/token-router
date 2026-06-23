# Token Router — Windows Electron 接入包

本目录为 `make package-electron-win` 生成的 **Windows x64 Electron 嵌入** 分发包。

## 目录结构

```
token-router-electron-win32-x64/
├── README.md                      ← 本文件
├── VERSION                        ← token-router crate 版本
├── electron-builder.example.yml   ← electron-builder extraResources 示例
├── resources/
│   └── win32/x64/
│       └── token_router.dll       ← 拷贝到宿主项目的 resources/
├── ffi/
│   └── token_router.h             ← C ABI 头文件
├── config/
│   └── config.toml                ← 完整示例配置（edge + cloud）
├── docs/
│   ├── incept.md                  ← 完整接入指南
│   └── incept.html
└── example/
    ├── package.ffi-rs.json        ← 宿主推荐 npm 依赖（ffi-rs）
    └── smoke/                     ← koffi 冒烟测试（可选）
        ├── main.mjs
        └── package.json
```

## 快速接入

1. 将 `resources/win32/x64/token_router.dll` 复制到宿主项目：

   ```text
   your-electron-app/resources/win32/x64/token_router.dll
   ```

2. 宿主 `package.json` 增加依赖（见 `example/package.ffi-rs.json`）。

3. 按 `docs/incept.md` 实现 FFI 绑定、Admin HTTP 配置与 IPC。

4. `electron-builder.yml` 参考根目录 `electron-builder.example.yml` 配置 `extraResources`。

## 冒烟测试

需先将 DLL 置于包内 `resources/win32/x64/`，然后：

```bash
cd example/smoke
npm install
npm start
```

## 数据目录

Gateway 默认配置与数据：`%USERPROFILE%\.token-router\config.toml`
