package com.eddlai.phonedesk;

import android.app.Activity;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.content.pm.ShortcutInfo;
import android.content.pm.ShortcutManager;
import android.graphics.Color;
import android.graphics.Rect;
import android.graphics.drawable.Icon;
import android.os.Bundle;
import android.view.Display;
import android.view.DisplayCutout;
import android.view.Gravity;
import android.view.RoundedCorner;
import android.view.WindowInsets;
import android.view.WindowMetrics;
import android.widget.TextView;

import rikka.shizuku.Shizuku;

/**
 * The "電腦模式" switch. On the phone screen it enters desktop mode and stays behind the desk
 * as its landscape host; opened from inside the desk it switches back to phone mode.
 */
public class MainActivity extends Activity {
    private static final int DESK_DPI = 240; // ponytail: fixed density, make it a setting if 240 feels wrong
    private static MainActivity host;

    private TextView status;
    private boolean insideDesk;
    private boolean requested;

    private final Shizuku.OnRequestPermissionResultListener permissionListener = (code, result) -> {
        if (result == PackageManager.PERMISSION_GRANTED) act();
        else show("需要授權 Shizuku 權限");
    };
    private final Shizuku.OnBinderReceivedListener binderListener = this::connectShizuku;
    private final Shizuku.OnBinderDeadListener binderDeadListener =
            () -> runOnUiThread(() -> show("Shizuku 沒有在執行\n請先在 Shizuku 裡啟動"));

    /** Called by the overlay when the desk closes. */
    static void finishHost() {
        MainActivity h = host;
        host = null;
        if (h != null) h.finishAndRemoveTask();
    }

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        Display display = getDisplay();
        insideDesk = display != null && display.getDisplayId() != Display.DEFAULT_DISPLAY;

        status = new TextView(this);
        status.setTextColor(Color.WHITE);
        status.setTextSize(18);
        status.setGravity(Gravity.CENTER);
        status.setBackgroundColor(Color.BLACK);
        setContentView(status);

        // While the desk runs it covers the phone screen, so any launch of "電腦模式" came from
        // inside the desk (Android may still place it on display 0): it means back to phone mode.
        if (insideDesk || DeskOverlayService.instance != null) {
            android.util.Log.i("PhoneDesk", "switch opened while desk runs (display "
                    + (display == null ? -1 : display.getDisplayId()) + "): exiting");
            if (DeskOverlayService.instance != null) DeskOverlayService.instance.exit();
            finishAndRemoveTask();
            return;
        }
        host = this;
        offerHomeShortcut();
        show("進入電腦模式…");
        Shizuku.addRequestPermissionResultListener(permissionListener);
        Shizuku.addBinderDeadListener(binderDeadListener);
        Shizuku.addBinderReceivedListenerSticky(binderListener);
    }

    /**
     * Tapping "電腦模式" inside the desk reaches this existing host task (Android reuses it
     * instead of starting a new instance there), so a relaunch while the desk runs means exit.
     */
    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        android.util.Log.i("PhoneDesk", "onNewIntent, overlay=" + (DeskOverlayService.instance != null));
        if (DeskOverlayService.instance != null) DeskOverlayService.instance.exit();
    }

    @Override
    protected void onDestroy() {
        Shizuku.removeBinderReceivedListener(binderListener);
        Shizuku.removeBinderDeadListener(binderDeadListener);
        Shizuku.removeRequestPermissionResultListener(permissionListener);
        if (host == this) host = null;
        super.onDestroy();
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        act(); // measuring needs the landscape relayout
    }

    private void connectShizuku() {
        if (Shizuku.isPreV11()) {
            show("Shizuku 版本太舊");
        } else if (Shizuku.checkSelfPermission() == PackageManager.PERMISSION_GRANTED) {
            act();
        } else {
            Shizuku.requestPermission(0);
        }
    }

    private void act() {
        if (requested || insideDesk || host != this) return;
        if (!Shizuku.pingBinder() || Shizuku.checkSelfPermission() != PackageManager.PERMISSION_GRANTED) return;
        WindowMetrics metrics = getWindowManager().getCurrentWindowMetrics();
        Rect bounds = metrics.getBounds();
        if (bounds.width() < bounds.height()) return; // wait for landscape
        requested = true;
        Rect m = safeMargins(metrics.getWindowInsets());
        getSharedPreferences("desk", MODE_PRIVATE).edit()
                .putInt("w", bounds.width()).putInt("h", bounds.height()).putInt("dpi", DESK_DPI)
                .putInt("ml", m.left).putInt("mt", m.top).putInt("mr", m.right).putInt("mb", m.bottom)
                .putBoolean("pending", true).apply();
        if (DeskOverlayService.instance != null) {
            DeskOverlayService.instance.start();
            return;
        }
        status.postDelayed(() -> {
            if (host == this && DeskOverlayService.instance == null) {
                show("電腦模式沒有啟動\nShizuku 可能沒有回應，請在 Shizuku 裡重新啟動後再試\n\n按返回鍵離開");
            }
        }, 10000);
        String component = getPackageName() + "/" + DeskOverlayService.class.getName();
        DeskConnection.whenReady(() -> new Thread(() -> {
            try {
                DeskConnection.get().setAccessibility(true, component);
            } catch (Exception e) {
                runOnUiThread(() -> show("電腦模式啟動失敗：" + e.getMessage()));
            }
        }).start());
    }

    /** Keeps the desk clear of the rounded screen corners and the camera cutout. */
    private static Rect safeMargins(WindowInsets insets) {
        int radius = 0;
        for (int pos : new int[]{RoundedCorner.POSITION_TOP_LEFT, RoundedCorner.POSITION_TOP_RIGHT,
                RoundedCorner.POSITION_BOTTOM_LEFT, RoundedCorner.POSITION_BOTTOM_RIGHT}) {
            RoundedCorner c = insets.getRoundedCorner(pos);
            if (c != null) radius = Math.max(radius, c.getRadius());
        }
        // ponytail: half the corner radius clears the status-bar text; tune if icons still clip
        int side = radius / 2;
        Rect m = new Rect(side, 0, side, 0);
        DisplayCutout cutout = insets.getDisplayCutout();
        if (cutout != null) {
            m.left = Math.max(m.left, cutout.getSafeInsetLeft());
            m.right = Math.max(m.right, cutout.getSafeInsetRight());
            m.top = Math.max(m.top, cutout.getSafeInsetTop());
            m.bottom = Math.max(m.bottom, cutout.getSafeInsetBottom());
        }
        return m;
    }

    /** Asks the launcher once to pin a "電腦模式" shortcut to the home screen. */
    private void offerHomeShortcut() {
        SharedPreferences prefs = getSharedPreferences("desk", MODE_PRIVATE);
        if (prefs.getBoolean("shortcut_offered", false)) return;
        ShortcutManager sm = getSystemService(ShortcutManager.class);
        if (sm == null || !sm.isRequestPinShortcutSupported()) return;
        ShortcutInfo info = new ShortcutInfo.Builder(this, "desk")
                .setShortLabel(getString(R.string.app_name))
                .setIcon(Icon.createWithResource(this, R.mipmap.ic_launcher))
                .setIntent(new Intent(this, MainActivity.class).setAction(Intent.ACTION_MAIN))
                .build();
        sm.requestPinShortcut(info, null);
        prefs.edit().putBoolean("shortcut_offered", true).apply();
    }

    private void show(String text) {
        status.setText(text);
    }
}
