package com.velocenetwork.mobile

import com.google.gson.Gson
import com.google.gson.reflect.TypeToken

data class NodeStatus(
    val is_running: Boolean = false,
    val machine_id: String = "",
    val machine_name: String = "",
    val peer_count: Int = 0,
    val mesh_port: Int = 10550,
    val dns_port: Int = 5354,
    val socks_port: Int = 1055,
    val uptime_secs: Long = 0L
)

data class PeerInfo(
    val peer_id: String,
    val peer_name: String,
    val latency_ms: Long,
    val tx_bytes: Long,
    val rx_bytes: Long,
    val hostnames: List<String>
)

object VeloceNative {
    init {
        try {
            System.loadLibrary("veloce_mobile")
        } catch (e: UnsatisfiedLinkError) {
            e.printStackTrace()
        }
    }

    private val gson = Gson()

    external fun startNode(dataDir: String, joinCode: String?, meshPort: Int): Boolean
    external fun stopNode(): Boolean
    external fun isRunning(): Boolean
    external fun getNodeStatus(): String
    external fun getPeers(): String
    external fun getMeshKv(key: String): String?
    external fun putMeshKv(key: String, value: String): Boolean
    external fun resolveHostname(hostname: String): String?
    external fun getMetrics(): String

    fun parseStatus(): NodeStatus {
        val json = getNodeStatus()
        return try {
            gson.fromJson(json, NodeStatus::class.java)
        } catch (e: Exception) {
            NodeStatus()
        }
    }

    fun parsePeers(): List<PeerInfo> {
        val json = getPeers()
        return try {
            val listType = object : TypeToken<List<PeerInfo>>() {}.type
            gson.fromJson(json, listType) ?: emptyList()
        } catch (e: Exception) {
            emptyList()
        }
    }
}
