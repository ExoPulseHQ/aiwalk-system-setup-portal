package com.aiwalk.setup

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.provider.Settings
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  // The aiwalk:// link this app was opened with (from Obsidian), kept until the page has dealt with it.
  @Volatile private var link: String? = null
  private fun keep(intent: Intent?) { intent?.data?.takeIf { it.scheme == "aiwalk" }?.let { link = it.toString() } }

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    keep(intent)
  }

  // already running (the activity is singleTask): the page looks for the link when it comes to the front
  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    keep(intent)
  }

  // The vault goes to Documents/aIwalk, the only kind of folder Obsidian for Android can open. Writing there needs
  // "All files access" (Android 11 and later) or the storage permission (before). The page asks through this.
  override fun onWebViewCreate(webView: WebView) {
    webView.addJavascriptInterface(Files(), "aiwalkFiles")
  }

  inner class Files {
    /** A long job starts: the service that keeps it alive off screen. `file` is where the job writes its words. */
    @JavascriptInterface
    fun work(text: String, file: String) {
      runOnUiThread {
        // Android 13 and later show the notification only when allowed; the job runs on either way
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
          requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 2)
        }
        val start = Intent(this@MainActivity, WorkService::class.java).putExtra("text", text).putExtra("file", file)
        try { if (Build.VERSION.SDK_INT >= 26) startForegroundService(start) else startService(start) } catch (e: Exception) {}
      }
    }

    @JavascriptInterface
    fun link(): String = link ?: ""

    @JavascriptInterface
    fun linkDone() { link = null }

    /** Back to Obsidian on the file that was fetched. Nothing but an obsidian:// address is opened. */
    @JavascriptInterface
    fun open(url: String) {
      if (!url.startsWith("obsidian://")) return
      runOnUiThread { try { startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) } catch (e: Exception) {} }
    }

    @JavascriptInterface
    fun allowed(): Boolean =
      if (Build.VERSION.SDK_INT >= 30) Environment.isExternalStorageManager()
      else checkSelfPermission(Manifest.permission.WRITE_EXTERNAL_STORAGE) == PackageManager.PERMISSION_GRANTED

    /** Opens Android's "All files access" screen for this app (before Android 11: the permission prompt). */
    @JavascriptInterface
    fun ask() {
      runOnUiThread {
        if (Build.VERSION.SDK_INT >= 30) {
          try {
            startActivity(Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION, Uri.parse("package:$packageName")))
          } catch (e: Exception) {   // a phone without the per-app screen: the list of all apps
            startActivity(Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION))
          }
        } else requestPermissions(arrayOf(Manifest.permission.WRITE_EXTERNAL_STORAGE), 1)
      }
    }
  }
}
