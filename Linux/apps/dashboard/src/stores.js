import { writable } from 'svelte/store';

export const connected       = writable(false);
export const nodes           = writable([]);
export const templates       = writable([]);
export const meshInfo        = writable(null);
// nodeId → [{stream, text, ts}]
export const logLines        = writable({});
// nodeId → {cpu_ms, mem_bytes, cpu_pct, mem_mb}
export const resources       = writable({});
// nodeId → [{ts, cpu_ms, mem_bytes}]  — max 120 entries
export const resourceHistory = writable({});
// Latest TrafficStatsMsg from "traffic-update" event
export const traffic         = writable(null);
// Last 60 TrafficStatsMsg snapshots (2-min window at 2s intervals)
export const trafficHistory  = writable([]);
// PolicyRulesMsg
export const policy          = writable(null);
// nodeId → {x, y} canvas positions; persisted to localStorage
export const topoPositions   = writable(
    JSON.parse(localStorage.getItem('veloce-topo-pos') || '{}')
);

// Persist topoPositions to localStorage whenever it changes
topoPositions.subscribe(pos => {
    try { localStorage.setItem('veloce-topo-pos', JSON.stringify(pos)); } catch {}
});
