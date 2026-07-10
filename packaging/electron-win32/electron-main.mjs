import koffi from "koffi";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const packageRoot = path.join(__dirname, "../..");

function archDir() {
  if (process.arch === "x64") return "x64";
  if (process.arch === "arm64") return "arm64";
  return process.arch;
}

function libraryPath() {
  const platform =
    process.platform === "win32"
      ? "win32"
      : process.platform === "darwin"
        ? "darwin"
        : "linux";
  const fileName =
    process.platform === "win32"
      ? "token_router.dll"
      : process.platform === "darwin"
        ? "libtoken_router.dylib"
        : "libtoken_router.so";

  const packaged = path.join(
    packageRoot,
    "resources",
    platform,
    archDir(),
    fileName,
  );
  if (existsSync(packaged)) return packaged;

  const devBase = path.join(__dirname, "../../../target/release");
  const dev = path.join(devBase, fileName);
  if (existsSync(dev)) return dev;

  throw new Error(`library not found (tried ${packaged} and ${dev})`);
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
const dll = libraryPath();

console.log("library:", dll);
console.log("token_router version:", token_router_version());

const startCode = token_router_start(homeDir, port, errorBuf, errorBuf.length);
if (startCode !== TOKEN_OK) {
  console.error("start failed:", errorBuf.toString("utf8").replace(/\0.*$/, ""));
  process.exit(1);
}

const urlLen = token_router_gateway_url(urlBuf, urlBuf.length);
if (urlLen < 0) {
  console.error("gateway_url failed:", urlBuf.toString("utf8").replace(/\0.*$/, ""));
  process.exit(1);
}
console.log("gateway url:", urlBuf.toString("utf8").replace(/\0.*$/, ""));
console.log("gateway running:", Boolean(token_router_is_running()));

setTimeout(() => {
  const stopCode = token_router_stop(errorBuf, errorBuf.length);
  if (stopCode !== TOKEN_OK) {
    console.error("stop failed:", errorBuf.toString("utf8").replace(/\0.*$/, ""));
    process.exit(1);
  }
  console.log("stopped");
}, 2000);
