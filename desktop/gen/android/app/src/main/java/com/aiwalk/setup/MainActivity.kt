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
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }

  // The vault goes to Documents/aIwalk, the only kind of folder Obsidian for Android can open. Writing there needs
  // "All files access" (Android 11 and later) or the storage permission (before). The page asks through this.
  override fun onWebViewCreate(webView: WebView) {
    webView.addJavascriptInterface(Files(), "aiwalkFiles")
  }

  inner class Files {
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
