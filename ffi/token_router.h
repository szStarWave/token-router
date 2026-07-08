#ifndef TOKEN_ROUTER_H
#define TOKEN_ROUTER_H

#include <stddef.h>
#include <stdint.h>

#ifdef _WIN32
#  ifdef TOKEN_ROUTER_EXPORTS
#    define TOKEN_ROUTER_API __declspec(dllexport)
#  else
#    define TOKEN_ROUTER_API __declspec(dllimport)
#  endif
#else
#  define TOKEN_ROUTER_API
#endif

#ifdef __cplusplus
extern "C" {
#endif

#define TOKEN_OK 0
#define TOKEN_ERR_ALREADY_RUNNING 1
#define TOKEN_ERR_NOT_RUNNING 2
#define TOKEN_ERR_INVALID_ARG 3
#define TOKEN_ERR_INTERNAL 4

TOKEN_ROUTER_API const char *token_router_version(void);

TOKEN_ROUTER_API int32_t token_router_start(
    const char *home_dir,
    uint16_t port,
    char *error_out,
    size_t error_out_len);

TOKEN_ROUTER_API int32_t token_router_stop(char *error_out, size_t error_out_len);

TOKEN_ROUTER_API int32_t token_router_is_running(void);

TOKEN_ROUTER_API int32_t token_router_gateway_url(char *url_out, size_t url_out_len);

#ifdef __cplusplus
}
#endif

#endif /* TOKEN_ROUTER_H */
