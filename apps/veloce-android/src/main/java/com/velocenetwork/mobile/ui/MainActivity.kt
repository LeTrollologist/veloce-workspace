package com.velocenetwork.mobile.ui

import android.app.Activity
import android.content.Intent
import android.net.VpnService
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.velocenetwork.mobile.NodeStatus
import com.velocenetwork.mobile.PeerInfo
import com.velocenetwork.mobile.VeloceNative
import com.velocenetwork.mobile.VeloceVpnService
import kotlinx.coroutines.delay

class MainActivity : ComponentActivity() {

    private var pendingJoinCode: String? = null

    private val vpnPermissionLauncher = registerForActivityResult(
        ActivityResultContracts.StartActivityForResult()
    ) { result ->
        if (result.resultCode == Activity.RESULT_OK) {
            startVpnService(pendingJoinCode)
        } else {
            Toast.makeText(this, "VPN permission denied", Toast.LENGTH_SHORT).show()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MaterialTheme(
                colorScheme = darkColorScheme(
                    primary = Color(0xFF00E5FF),
                    secondary = Color(0xFF7C4DFF),
                    background = Color(0xFF0F141C),
                    surface = Color(0xFF1B2230),
                    onSurface = Color(0xFFECEFF1)
                )
            ) {
                Surface(
                    modifier = Modifier.fillMaxSize(),
                    color = MaterialTheme.colorScheme.background
                ) {
                    VeloceMainScreen(
                        onConnectToggle = { isConnecting, joinCode ->
                            if (isConnecting) {
                                requestAndStartVpn(joinCode)
                            } else {
                                stopVpnService()
                            }
                        }
                    )
                }
            }
        }
    }

    private fun requestAndStartVpn(joinCode: String?) {
        pendingJoinCode = joinCode
        val prepareIntent = VpnService.prepare(this)
        if (prepareIntent != null) {
            vpnPermissionLauncher.launch(prepareIntent)
        } else {
            startVpnService(joinCode)
        }
    }

    private fun startVpnService(joinCode: String?) {
        val intent = Intent(this, VeloceVpnService::class.java).apply {
            action = VeloceVpnService.ACTION_CONNECT
            putExtra(VeloceVpnService.EXTRA_JOIN_CODE, joinCode)
        }
        startService(intent)
    }

    private fun stopVpnService() {
        val intent = Intent(this, VeloceVpnService::class.java).apply {
            action = VeloceVpnService.ACTION_DISCONNECT
        }
        startService(intent)
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun VeloceMainScreen(
    onConnectToggle: (Boolean, String?) -> Unit
) {
    var selectedTab by remember { mutableStateOf(0) }
    var status by remember { mutableStateOf(NodeStatus()) }
    var peers by remember { mutableStateOf(emptyList<PeerInfo>()) }
    var joinCodeInput by remember { mutableStateOf("") }
    var kvKeyInput by remember { mutableStateOf("") }
    var kvValInput by remember { mutableStateOf("") }
    var kvResult by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(Unit) {
        while (true) {
            status = VeloceNative.parseStatus()
            if (status.is_running) {
                peers = VeloceNative.parsePeers()
            } else {
                peers = emptyList()
            }
            delay(1500)
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text("⚡ VeloceNetwork", fontWeight = FontWeight.Bold)
                        Spacer(modifier = Modifier.width(8.dp))
                        Badge(
                            containerColor = if (status.is_running) Color(0xFF00E676) else Color(0xFFFF5252)
                        ) {
                            Text(if (status.is_running) "ONLINE" else "OFFLINE", color = Color.Black)
                        }
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = MaterialTheme.colorScheme.surface
                )
            )
        },
        bottomBar = {
            NavigationBar(containerColor = MaterialTheme.colorScheme.surface) {
                NavigationBarItem(
                    selected = selectedTab == 0,
                    onClick = { selectedTab = 0 },
                    icon = { Icon(Icons.Default.Dashboard, contentDescription = "Dashboard") },
                    label = { Text("Mesh") }
                )
                NavigationBarItem(
                    selected = selectedTab == 1,
                    onClick = { selectedTab = 1 },
                    icon = { Icon(Icons.Default.People, contentDescription = "Peers") },
                    label = { Text("Peers (${peers.size})") }
                )
                NavigationBarItem(
                    selected = selectedTab == 2,
                    onClick = { selectedTab = 2 },
                    icon = { Icon(Icons.Default.Storage, contentDescription = "KV Store") },
                    label = { Text("Replicated KV") }
                )
            }
        }
    ) { padding ->
        Box(modifier = Modifier.padding(padding)) {
            when (selectedTab) {
                0 -> DashboardTab(
                    status = status,
                    joinCode = joinCodeInput,
                    onJoinCodeChange = { joinCodeInput = it },
                    onConnect = { onConnectToggle(true, if (joinCodeInput.isNotBlank()) joinCodeInput else null) },
                    onDisconnect = { onConnectToggle(false, null) }
                )
                1 -> PeersTab(peers = peers)
                2 -> KvTab(
                    key = kvKeyInput,
                    value = kvValInput,
                    result = kvResult,
                    onKeyChange = { kvKeyInput = it },
                    onValChange = { kvValInput = it },
                    onGet = {
                        kvResult = VeloceNative.getMeshKv(kvKeyInput) ?: "[Key not found]"
                    },
                    onPut = {
                        val ok = VeloceNative.putMeshKv(kvKeyInput, kvValInput)
                        kvResult = if (ok) "[Set successful]" else "[Set failed]"
                    }
                )
            }
        }
    }
}

@Composable
fun DashboardTab(
    status: NodeStatus,
    joinCode: String,
    onJoinCodeChange: (String) -> Unit,
    onConnect: () -> Unit,
    onDisconnect: () -> Unit
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(16.dp)
    ) {
        // Main Connect Card
        Card(
            modifier = Modifier.fillMaxWidth(),
            colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
            shape = RoundedCornerShape(16.dp)
        ) {
            Column(
                modifier = Modifier.padding(20.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(12.dp)
            ) {
                Button(
                    onClick = {
                        if (status.is_running) onDisconnect() else onConnect()
                    },
                    modifier = Modifier.size(120.dp),
                    shape = CircleShape,
                    colors = ButtonDefaults.buttonColors(
                        containerColor = if (status.is_running) Color(0xFFFF5252) else Color(0xFF00E5FF)
                    )
                ) {
                    Icon(
                        imageVector = if (status.is_running) Icons.Default.PowerSettingsNew else Icons.Default.VpnKey,
                        contentDescription = "Power",
                        tint = Color.Black,
                        modifier = Modifier.size(48.dp)
                    )
                }

                Text(
                    text = if (status.is_running) "P2P Mesh Active" else "Ready to Connect",
                    fontSize = 18.sp,
                    fontWeight = FontWeight.Bold
                )

                if (status.is_running) {
                    Text(
                        text = "Node: ${status.machine_name}.vln",
                        fontFamily = FontFamily.Monospace,
                        color = Color(0xFF80DEEA)
                    )
                }
            }
        }

        // Quick Join Code Input
        if (!status.is_running) {
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface)
            ) {
                Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Join Mesh Network", fontWeight = FontWeight.SemiBold)
                    OutlinedTextField(
                        value = joinCode,
                        onValueChange = onJoinCodeChange,
                        label = { Text("Paste VM3 Join Code") },
                        placeholder = { Text("VM3-...") },
                        modifier = Modifier.fillMaxWidth(),
                        singleLine = true
                    )
                }
            }
        }

        // Stats Card
        if (status.is_running) {
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface)
            ) {
                Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Runtime Telemetry", fontWeight = FontWeight.SemiBold)
                    Divider(color = Color.DarkGray)
                    Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                        Text("Connected Peers:")
                        Text("${status.peer_count}", fontWeight = FontWeight.Bold, color = Color(0xFF00E5FF))
                    }
                    Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                        Text("Userspace DNS:")
                        Text("127.0.0.1:${status.dns_port}", fontFamily = FontFamily.Monospace)
                    }
                    Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                        Text("SOCKS5 Proxy:")
                        Text("127.0.0.1:${status.socks_port}", fontFamily = FontFamily.Monospace)
                    }
                    Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                        Text("Uptime:")
                        Text("${status.uptime_secs}s")
                    }
                }
            }
        }
    }
}

@Composable
fun PeersTab(peers: List<PeerInfo>) {
    if (peers.isEmpty()) {
        Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            Text("No peers connected yet.\nJoin a mesh with another machine!", color = Color.Gray)
        }
    } else {
        LazyColumn(
            modifier = Modifier.fillMaxSize().padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp)
        ) {
            items(peers) { peer ->
                Card(
                    modifier = Modifier.fillMaxWidth(),
                    colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface)
                ) {
                    Column(modifier = Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                            Text(peer.peer_name, fontWeight = FontWeight.Bold, fontSize = 16.sp)
                            Text("${peer.latency_ms} ms", color = Color(0xFF00E676), fontWeight = FontWeight.SemiBold)
                        }
                        Text("ID: ${peer.peer_id}", fontSize = 12.sp, color = Color.Gray, fontFamily = FontFamily.Monospace)
                        if (peer.hostnames.isNotEmpty()) {
                            Text("Services: ${peer.hostnames.joinToString(", ")}", fontSize = 13.sp, color = Color(0xFF80DEEA))
                        }
                        Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                            Text("TX: ${peer.tx_bytes} B", fontSize = 12.sp, color = Color.LightGray)
                            Text("RX: ${peer.rx_bytes} B", fontSize = 12.sp, color = Color.LightGray)
                        }
                    }
                }
            }
        }
    }
}

@Composable
fun KvTab(
    key: String,
    value: String,
    result: String?,
    onKeyChange: (String) -> Unit,
    onValChange: (String) -> Unit,
    onGet: () -> Unit,
    onPut: () -> Unit
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        Text("Replicated Mesh Key-Value Store", fontWeight = FontWeight.Bold, fontSize = 16.sp)
        Text("Keys and values sync automatically across all connected mesh peers in real-time.", fontSize = 13.sp, color = Color.Gray)

        OutlinedTextField(
            value = key,
            onValueChange = onKeyChange,
            label = { Text("Key") },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true
        )

        OutlinedTextField(
            value = value,
            onValueChange = onValChange,
            label = { Text("Value") },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true
        )

        Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Button(onClick = onGet, modifier = Modifier.weight(1f)) {
                Text("GET")
            }
            Button(onClick = onPut, modifier = Modifier.weight(1f)) {
                Text("SET")
            }
        }

        if (result != null) {
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface)
            ) {
                Column(modifier = Modifier.padding(12.dp)) {
                    Text("Result:", fontWeight = FontWeight.SemiBold, fontSize = 12.sp, color = Color.Gray)
                    Text(result, fontFamily = FontFamily.Monospace, fontSize = 14.sp, color = Color(0xFF00E5FF))
                }
            }
        }
    }
}
