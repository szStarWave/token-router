import koffi from "koffi";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const base = path.join(__dirname, "../../target/release");

function libraryPath() {
  if (process.platform === "win32") {
    return path.join(base, "token_router.dll");
  }
  if (process.platform === "darwin") {
    return path.join(base, "libtoken_router.dylib");
  }
  return path.join(base, "libtoken_router.so");
}

const lib = koffi.load(libraryPath());

const TOKEN_OK = 0;
const token_router_version = lib.func("const char *token_router_version()");
const token_router_start = lib.func(
  "int32 token_router_start(const char *config_path, _Out_ char *error_out, size_t error_out_len)",
);
const token_router_stop = lib.func(
  "int32 token_router_stop(_Out_ char *error_out, size_t error_out_len)",
);
const token_router_is_running = lib.func("int32 token_router_is_running()");
const token_router_gateway_url = lib.func(
  "int32 token_router_gateway_url(_Out_ char *url_out, size_t url_out_len)",
);

const configPath = process.argv[2] ?? null;
const errorBuf = Buffer.alloc(4096);
const urlBuf = Buffer.alloc(256);

console.log("library:", libraryPath());
console.log("token_router version:", token_router_version());

const startCode = token_router_start(configPath, errorBuf, errorBuf.length);
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
