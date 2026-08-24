package com.velocenetwork.mobile

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import android.net.VpnService
import android.os.Build
import android.os.ParcelFileDescriptor
import androidx.core.app.NotificationCompat
import com.velocenetwork.mobile.ui.MainActivity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

class VeloceVpnService : VpnService() {

    private var vpnInterface: ParcelFileDescriptor? = null
    private val serviceJob = Job()
    private val serviceScope = CoroutineScope(Dispatchers.IO + serviceJob)

    companion object {
        const val ACTION_CONNECT = "com.velocenetwork.mobile.CONNECT"
        const val ACTION_DISCONNECT = "com.velocenetwork.mobile.DISCONNECT"
        const val EXTRA_JOIN_CODE = "extra_join_code"
        const val CHANNEL_ID = "veloce_vpn_channel"
        const val NOTIFICATION_ID = 1055
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val action = intent?.action
        if (action == ACTION_DISCONNECT) {
            disconnect()
            stopSelf()
            return START_NOT_STICKY
        }

        val joinCode = intent?.getStringExtra(EXTRA_JOIN_CODE)
        connect(joinCode)
        return START_STICKY
    }

    private fun connect(joinCode: String?) {
        createNotificationChannel()
        val notification = createNotification("VeloceNetwork Mesh Active")
        startForeground(NOTIFICATION_ID, notification)

        val dataDir = filesDir.absolutePath
        val started = VeloceNative.startNode(dataDir, joinCode, 10550)

        if (started) {
            try {
                val builder = Builder()
                    .setSession("VeloceNetwork")
                    .addAddress("100.64.0.2", 24)
                    .addRoute("100.64.0.0", 10)
                    .addDnsServer("127.0.0.1")
                    .setMtu(1400)
                    .setBlocking(false)

                vpnInterface = builder.establish()
            } catch (e: Exception) {
                e.printStackTrace()
            }
        }

        // Monitoring loop
        serviceScope.launch {
            while (isActive && VeloceNative.isRunning()) {
                delay(3000)
            }
        }
    }

    private fun disconnect() {
        try {
            vpnInterface?.close()
            vpnInterface = null
        } catch (e: Exception) {
            e.printStackTrace()
        }
        VeloceNative.stopNode()
        stopForeground(STOP_FOREGROUND_REMOVE)
    }

    override fun onDestroy() {
        serviceJob.cancel()
        disconnect()
        super.onDestroy()
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "VeloceNetwork Mesh Status",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "Shows live P2P mesh and VPN connection state"
            }
            val manager = getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(channel)
        }
    }

    private fun createNotification(contentText: String): Notification {
        val intent = Intent(this, MainActivity::class.java)
        val pendingIntent = PendingIntent.getActivity(
            this, 0, intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("VeloceNetwork Mesh")
            .setContentText(contentText)
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .setContentIntent(pendingIntent)
            .setOngoing(true)
            .build()
    }
}
