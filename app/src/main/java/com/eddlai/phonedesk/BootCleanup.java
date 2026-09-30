package com.eddlai.phonedesk;

import android.content.BroadcastReceiver;
import android.content.ContentResolver;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.provider.Settings;
import android.util.Log;

/**
 * Settings survive a reboot but the desk does not: if the phone restarted (or died) while
 * desktop mode was on, undo it at boot, before unlock. Needs WRITE_SECURE_SETTINGS, which the
 * shell service grants when desktop mode is first entered.
 */
public class BootCleanup extends BroadcastReceiver {
    private static final String TAG = "PhoneDesk";

    /** State kept in device-protected storage so it is readable before the first unlock. */
    static SharedPreferences state(Context c) {
        return c.createDeviceProtectedStorageContext().getSharedPreferences("desk_state", Context.MODE_PRIVATE);
    }

    /** Remembers that the desk is on, plus the rotation settings to restore. */
    static void markOn(Context c) {
        ContentResolver r = c.getContentResolver();
        state(c).edit()
                .putBoolean("on", true)
                .putInt("accel", Settings.System.getInt(r, Settings.System.ACCELEROMETER_ROTATION, 1))
                .putInt("rotation", Settings.System.getInt(r, Settings.System.USER_ROTATION, 0))
                .commit();
    }

    static void markOff(Context c) {
        state(c).edit().putBoolean("on", false).commit();
    }

    @Override
    public void onReceive(Context context, Intent intent) {
        SharedPreferences s = state(context);
        if (!s.getBoolean("on", false)) return;
        Log.w(TAG, "desk was on at shutdown, cleaning up");
        ContentResolver r = context.getContentResolver();
        try {
            Settings.Global.putString(r, "overlay_display_devices", "none");
            String svc = Settings.Secure.getString(r, "enabled_accessibility_services");
            if (svc != null) {
                String mine = context.getPackageName() + "/" + DeskOverlayService.class.getName();
                StringBuilder kept = new StringBuilder();
                for (String part : svc.split(":")) {
                    if (part.isEmpty() || part.equals(mine)) continue;
                    if (kept.length() > 0) kept.append(':');
                    kept.append(part);
                }
                Settings.Secure.putString(r, "enabled_accessibility_services", kept.toString());
            }
            Settings.System.putInt(r, Settings.System.ACCELEROMETER_ROTATION, s.getInt("accel", 1));
            Settings.System.putInt(r, Settings.System.USER_ROTATION, s.getInt("rotation", 0));
            markOff(context);
        } catch (SecurityException e) {
            Log.e(TAG, "boot cleanup lacks permission", e);
        }
    }
}
