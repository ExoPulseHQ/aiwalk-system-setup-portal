package com.aiwalk.setup

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import java.io.File

/**
 * Keeps a long job alive (downloading the vault, uploading changes): Android may pause or end an app that is not on
 * screen, and does not while one of its services shows a notification. The page starts this as the job starts. The
 * job writes what it is doing to a file (phonegit.rs, Working) and removes the file when it ends; this reads the
 * file for the notification's words and stops when the file is gone, so it ends with the job whether or not the
 * page is awake to say so.
 */
class WorkService : Service() {
  private val handler = Handler(Looper.getMainLooper())
  private var file: File? = null
  private var misses = 0
  private var shown = ""
  private var lock: PowerManager.WakeLock? = null

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    intent?.getStringExtra("file")?.let { file = File(it) }
    show(intent?.getStringExtra("text") ?: "Working")
    // the processor stays on with the screen off; let go when the job ends, and after six hours whatever happens
    if (lock == null) {
      lock = (getSystemService(POWER_SERVICE) as PowerManager).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "aiwalk:work")
        .apply { acquire(6 * 60 * 60 * 1000L) }
    }
    misses = -6   // the job writes its file a moment after the page starts this: about ten seconds of grace
    handler.removeCallbacksAndMessages(null)
    handler.postDelayed({ poll() }, EVERY)
    return START_NOT_STICKY
  }

  private fun poll() {
    val text = try { file?.takeIf { it.exists() }?.readText()?.trim() } catch (e: Exception) { null }
    if (text.isNullOrEmpty()) {
      if (++misses >= 2) { stopSelf(); return }
    } else {
      misses = 0
      if (text != shown) show(text)
    }
    handler.postDelayed({ poll() }, EVERY)
  }

  private fun show(text: String) {
    shown = text
    val manager = getSystemService(NOTIFICATION_SERVICE) as NotificationManager
    if (Build.VERSION.SDK_INT >= 26 && manager.getNotificationChannel(CHANNEL) == null) {
      // low importance: it is there to be seen when looked for, not to make a sound
      manager.createNotificationChannel(NotificationChannel(CHANNEL, "Work in progress", NotificationManager.IMPORTANCE_LOW))
    }
    val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
    val note = NotificationCompat.Builder(this, CHANNEL)
      .setSmallIcon(android.R.drawable.stat_notify_sync)
      .setContentTitle(getString(R.string.app_name))
      .setContentText(text)
      .setOngoing(true).setOnlyAlertOnce(true).setContentIntent(open)
      .build()
    ServiceCompat.startForeground(this, 1, note, if (Build.VERSION.SDK_INT >= 29) ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC else 0)
  }

  override fun onDestroy() {
    handler.removeCallbacksAndMessages(null)
    lock?.let { if (it.isHeld) it.release() }
    super.onDestroy()
  }

  companion object {
    const val CHANNEL = "work"
    const val EVERY = 1500L
  }
}
