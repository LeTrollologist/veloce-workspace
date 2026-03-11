/**
 * veloce_sdk.h
 * C public API for the VeloceCore runtime SDK.
 *
 * Link against:  veloce_sdk.dll  (Windows)
 *
 * Quick start:
 *
 *   #include "veloce_sdk.h"
 *
 *   VeloceHandle* h = veloce_connect("MyApp", "1.0.0");
 *   if (!h) { fprintf(stderr, "VeloceCore not running\n"); return 1; }
 *
 *   CVeloceNode node = {0};
 *   if (veloce_spawn_node(h, "worker", "C:\\worker.exe", &node) != 0) {
 *       fprintf(stderr, "spawn failed\n");
 *   }
 *
 *   veloce_register_host(h, "worker.vln", node.node_id, 8080, 0);
 *   // HTTP traffic to http://worker.vln:8080 via SOCKS5 now routes to localhost:8080
 *
 *   veloce_disconnect(h);
 */

#ifndef VELOCE_SDK_H
#define VELOCE_SDK_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stdint.h>

/* ── Opaque handle ─────────────────────────────────────────────────────────── */

/**
 * Opaque connection handle returned by veloce_connect().
 * Must be freed with veloce_disconnect().
 */
typedef struct VeloceHandle VeloceHandle;

/* ── Spawned node result ───────────────────────────────────────────────────── */

/**
 * Describes a successfully spawned node.
 * Filled in by veloce_spawn_node().
 */
typedef struct CVeloceNode {
    /** UUID as a null-terminated string:  "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx\0" */
    char     node_id[37];
    /** Win32 process ID of the spawned node. */
    uint32_t pid;
    /** Named pipe the node is reachable on (null-terminated, max 260 chars). */
    char     node_pipe[261];
} CVeloceNode;

/* ── Lifecycle ─────────────────────────────────────────────────────────────── */

/**
 * Connect to a running VeloceCore service and return a session handle.
 *
 * @param app_name    Human-readable name for this client (e.g. "MyApp").
 * @param sdk_version Semver version of your application (e.g. "1.0.0").
 * @return            Handle on success, NULL on failure.
 *                    Logs the error to the VeloceCore log on failure.
 *
 * Thread safety: the returned handle may be used from any thread after
 * creation, but individual calls are not re-entrant — use one handle
 * per thread, or serialise access externally.
 */
VeloceHandle* veloce_connect(const char* app_name, const char* sdk_version);

/**
 * Disconnect and free all resources associated with the handle.
 * Any nodes with auto_kill=true are killed when the handle is freed.
 *
 * @param handle  Must not be NULL; safe to call on a failed handle.
 */
void veloce_disconnect(VeloceHandle* handle);

/* ── Heartbeat ─────────────────────────────────────────────────────────────── */

/**
 * Ping VeloceCore.
 *
 * @return 0 on success, -1 on failure.
 */
int veloce_ping(VeloceHandle* handle);

/* ── Node management ───────────────────────────────────────────────────────── */

/**
 * Spawn a child process inside an isolated Job Object.
 *
 * The spawned process inherits the environment of VeloceCore plus:
 *   VELOCE_PIPE      = \\.\pipe\VeloceCore  (connect back to Core)
 *   VELOCE_NODE_ID   = <uuid>
 *   VELOCE_NODE_PIPE = \\.\pipe\VeloceNode-<uuid>
 *
 * @param handle      Active session handle.
 * @param app_name    Display name for this node.
 * @param executable  Full path to the executable.
 * @param out         Filled with node metadata on success. Must not be NULL.
 * @return            0 on success, -1 on failure.
 */
int veloce_spawn_node(
    VeloceHandle*  handle,
    const char*    app_name,
    const char*    executable,
    CVeloceNode*   out
);

/**
 * Kill a running node by its UUID string.
 *
 * @param handle      Active session handle.
 * @param node_id_str UUID as a null-terminated string.
 * @return            0 on success, -1 on failure.
 */
int veloce_kill_node(VeloceHandle* handle, const char* node_id_str);

/* ── VeloceNet ─────────────────────────────────────────────────────────────── */

/**
 * Register a *.vln hostname that routes to a local TCP port.
 *
 * After registration, DNS queries for `hostname` return 127.0.0.1,
 * and SOCKS5 connections to `hostname` are forwarded to `local_port`.
 *
 * @param handle       Active session handle.
 * @param hostname     The *.vln name (e.g. "myapp.vln").
 * @param node_id_str  UUID of the owning node, as a null-terminated string.
 * @param local_port   Local TCP port the node is listening on.
 * @param ttl_secs     TTL in seconds (0 = permanent until unregistered/killed).
 * @return             0 on success, -1 on failure.
 */
int veloce_register_host(
    VeloceHandle* handle,
    const char*   hostname,
    const char*   node_id_str,
    uint16_t      local_port,
    uint64_t      ttl_secs
);

/**
 * Unregister a *.vln hostname.
 *
 * @return 0 on success, -1 on failure.
 */
int veloce_unregister_host(VeloceHandle* handle, const char* hostname);

/**
 * Resolve a *.vln hostname to its local address string ("127.0.0.1:<port>").
 *
 * @param handle    Active session handle.
 * @param hostname  The hostname to resolve.
 * @param out_buf   Buffer to write the address into.
 * @param out_len   Size of out_buf (at least 22 bytes for "127.0.0.1:65535\0").
 * @return          0 on success (out_buf filled), -1 if not found or error.
 */
int veloce_resolve_host(
    VeloceHandle* handle,
    const char*   hostname,
    char*         out_buf,
    int           out_len
);

/* ── Registry ──────────────────────────────────────────────────────────────── */

/**
 * Read a raw byte value from the VeloceCore mmap registry.
 *
 * @param handle    Active session handle.
 * @param key       Null-terminated registry key.
 * @param out_buf   Buffer to write value into.
 * @param out_len   Size of out_buf on input; bytes written on output.
 * @return          0 on success (value found), 1 if key not found, -1 on error.
 */
int veloce_registry_get(
    VeloceHandle* handle,
    const char*   key,
    uint8_t*      out_buf,
    int*          out_len
);

/**
 * Write a raw byte value to the VeloceCore mmap registry.
 *
 * @param handle    Active session handle.
 * @param key       Null-terminated registry key.
 * @param value     Bytes to write.
 * @param value_len Number of bytes in value.
 * @return          0 on success, -1 on failure.
 */
int veloce_registry_set(
    VeloceHandle*  handle,
    const char*    key,
    const uint8_t* value,
    int            value_len
);

/* ── Version ───────────────────────────────────────────────────────────────── */

/**
 * Returns the compile-time SDK version string (e.g. "0.1.0").
 * The returned pointer is static — do not free it.
 */
const char* veloce_sdk_version(void);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* VELOCE_SDK_H */