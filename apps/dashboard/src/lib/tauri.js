/**
 * tauri.js — typed invoke() wrappers for all VeloceNetwork Dashboard commands.
 *
 * All functions return promises and propagate errors from the Tauri backend.
 */

import { invoke } from '@tauri-apps/api/core';

export const api = {
    // ── Connection ──────────────────────────────────────────────────────────
    connect:            ()                  => invoke('connect'),
    disconnect:         ()                  => invoke('disconnect'),
    ping:               ()                  => invoke('ping'),

    // ── Nodes ───────────────────────────────────────────────────────────────
    listNodes:          ()                  => invoke('list_nodes'),
    spawnNode:          (appName, exe)      => invoke('spawn_node', { appName, executable: exe }),
    killNode:           (nodeId)            => invoke('kill_node', { nodeId }),
    queryResources:     ()                  => invoke('query_resources'),

    // ── Logs ────────────────────────────────────────────────────────────────
    subscribeLogs:      (nodeId)            => invoke('subscribe_node_logs', { nodeId }),

    // ── Templates ───────────────────────────────────────────────────────────
    listTemplates:      ()                  => invoke('list_templates'),
    getTemplate:        (name)              => invoke('get_template', { name }),
    saveTemplate:       (spec)              => invoke('save_template', { spec }),
    deleteTemplate:     (name)              => invoke('delete_template', { name }),
    spawnFromTemplate:  (name)              => invoke('spawn_from_template', { name }),

    // ── VeloceNet ───────────────────────────────────────────────────────────
    registerHost:       (hostname, nodeId, localPort, ttlSecs) =>
                            invoke('register_host', { hostname, nodeId, localPort, ttlSecs }),
    unregisterHost:     (hostname)          => invoke('unregister_host', { hostname }),

    // ── Mesh ────────────────────────────────────────────────────────────────
    meshInfo:           ()                  => invoke('mesh_info'),
    meshConnect:        (joinCode)          => invoke('mesh_connect', { joinCode }),
    meshDisconnect:     (peerId)            => invoke('mesh_disconnect', { peerId }),
    meshPeers:          ()                  => invoke('mesh_peers'),

    // ── Traffic ─────────────────────────────────────────────────────────────
    trafficStats:       ()                  => invoke('traffic_stats'),

    // ── Policy ──────────────────────────────────────────────────────────────
    policyShow:         ()                  => invoke('policy_show'),
    policyReload:       ()                  => invoke('policy_reload_cmd'),

    // ── Updates ─────────────────────────────────────────────────────────────
    checkForUpdates:    ()                  => invoke('check_for_updates'),
    installUpdate:      ()                  => invoke('install_update'),
};
