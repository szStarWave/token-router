# sidecar 说明（§12 打包）

`externalBin: sidecar/token-router-v2` 要求 target-triple 后缀的二进制：

```text
src-tauri/sidecar/token-router-v2-x86_64-pc-windows-msvc.exe
```

## 生成方式（发布 CI / 本地）

```bash
# 1. 构建服务端
cargo build --release --manifest-path ../../Cargo.toml -p tr-server

# 2. 复制为 sidecar（Windows 示例；CI 按目标 triple 替换）
cp ../../target/release/token-router-v2.exe \
   sidecar/token-router-v2-x86_64-pc-windows-msvc.exe
```

或使用脚本：

```bash
node scripts/prepare-sidecar.mjs
```

打包产物（NSIS）会把 sidecar 安装到应用目录；supervisor 在运行时按
「应用目录 → workspace target/debug（开发期回退）」顺序定位
`token-router-v2`（见 `src/lib.rs::sidecar_binary`）。

## 签名（§12，发布 CI）

- Windows：`signtool sign /fd SHA256 /tr <tsa> installer.exe`（证书由 CI secret 提供）
- 更新 manifest：`version/platform/sha256/signature`，下载后校验再替换；
  失败保留旧版本（当前 MVP 手动更新，OTA 走后续迭代）。

## 升级不覆盖用户数据（§12）

安装器只替换二进制与前端资源；`config.toml`、DB、`admin.token`、日志
均在数据目录（`~/.token-router-v2`），不受升级影响。
