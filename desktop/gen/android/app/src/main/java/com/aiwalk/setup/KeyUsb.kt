package com.aiwalk.setup

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.hardware.usb.UsbConstants
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbDeviceConnection
import android.hardware.usb.UsbEndpoint
import android.hardware.usb.UsbInterface
import android.hardware.usb.UsbManager
import android.os.Build
import org.json.JSONArray
import org.json.JSONObject

/**
 * A FIDO security key on the phone's USB port, for sealed notes (sealed.rs, fidokey.rs). Rust speaks CTAPHID through
 * these calls (JNI): 64-byte reports out and in on the key's HID interface. A key's FIDO interface is the HID
 * interface with an interrupt endpoint each way (a YubiKey's keyboard interface has only one, inwards). Android asks
 * once per key whether this app may use it.
 */
object KeyUsb {
  private var app: Context? = null
  private class Open(val conn: UsbDeviceConnection, val iface: UsbInterface, val out: UsbEndpoint, val inp: UsbEndpoint)
  private val open = HashMap<String, Open>()

  @JvmStatic fun init(context: Context) { app = context.applicationContext }

  private fun manager(): UsbManager? = app?.getSystemService(Context.USB_SERVICE) as UsbManager?

  /** The FIDO interface of a device and its two endpoints, when it has one. */
  private fun fido(d: UsbDevice): Triple<UsbInterface, UsbEndpoint, UsbEndpoint>? {
    for (i in 0 until d.interfaceCount) {
      val f = d.getInterface(i)
      if (f.interfaceClass != UsbConstants.USB_CLASS_HID) continue
      var out: UsbEndpoint? = null; var inp: UsbEndpoint? = null
      for (e in 0 until f.endpointCount) {
        val ep = f.getEndpoint(e)
        if (ep.type != UsbConstants.USB_ENDPOINT_XFER_INT || ep.maxPacketSize != 64) continue
        if (ep.direction == UsbConstants.USB_DIR_OUT) out = ep else inp = ep
      }
      if (out != null && inp != null) return Triple(f, out, inp)
    }
    return null
  }

  /** The keys plugged in, as JSON: [{"id": device name, "name": maker and product}]. */
  @JvmStatic fun devices(): String {
    val list = JSONArray()
    manager()?.deviceList?.values?.filter { fido(it) != null }?.forEach {
      list.put(JSONObject().put("id", it.deviceName).put("name", "${it.manufacturerName ?: ""} ${it.productName ?: "Security key"}".trim()))
    }
    return list.toString()
  }

  /** Opens the key for this app: "" when ready, or what stops it (Android asks permission once; then try again). */
  @JvmStatic fun open(id: String): String {
    if (open.containsKey(id)) return ""
    val m = manager() ?: return "No USB on this phone."
    val d = m.deviceList[id] ?: return "That key is not plugged in any more."
    val (iface, out, inp) = fido(d) ?: return "That device is not a security key."
    if (!m.hasPermission(d)) {
      val flags = if (Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else 0
      m.requestPermission(d, PendingIntent.getBroadcast(app, 0, Intent("com.aiwalk.setup.USB").setPackage(app!!.packageName), flags))
      return "Allow aIwalk System Setup to use the key in Android's question, then try again."
    }
    val conn = m.openDevice(d) ?: return "Android would not open the key."
    if (!conn.claimInterface(iface, true)) { conn.close(); return "Another app is using the key." }
    open[id] = Open(conn, iface, out, inp)
    return ""
  }

  /** One 64-byte report to the key; false when it could not be sent. */
  @JvmStatic fun send(id: String, report: ByteArray): Boolean {
    val o = open[id] ?: return false
    return o.conn.bulkTransfer(o.out, report, report.size, 2000) == report.size
  }

  /** One 64-byte report from the key, waiting up to `ms`; null when none came. */
  @JvmStatic fun recv(id: String, ms: Int): ByteArray? {
    val o = open[id] ?: return null
    val b = ByteArray(64)
    return if (o.conn.bulkTransfer(o.inp, b, 64, ms) == 64) b else null
  }

  @JvmStatic fun close(id: String) {
    open.remove(id)?.let { it.conn.releaseInterface(it.iface); it.conn.close() }
  }
}
