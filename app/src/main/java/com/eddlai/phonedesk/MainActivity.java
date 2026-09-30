package com.eddlai.phonedesk;

import android.app.Activity;
import android.content.ComponentName;
import android.content.Intent;
import android.content.ServiceConnection;
import android.content.SharedPreferences;
import android.content.pm.ShortcutInfo;
import android.content.pm.ShortcutManager;
import android.graphics.drawable.Icon;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.os.Bundle;
import android.os.IBinder;
import android.os.RemoteException;
import android.util.Log;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.Surface;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.widget.FrameLayout;
import android.widget.TextView;

import rikka.shizuku.Shizuku;

/**
 * Shows a trusted virtual display full screen on the phone. With
 * force_desktop_mode_on_external_displays=1 Android treats it as an external monitor and
 * runs the native desktop (taskbar + freeform windows) on it.
 */
public class MainActivity extends Activity implements SurfaceHolder.Callback {
    private static final String TAG = "PhoneDesk";
    private static final int DESK_DPI = 240; // ponytail: fixed density, make it a setting if 240 feels wrong

    private SurfaceView surfaceView;
    private TextView status;
    private IDeskService desk;
    private Surface surface;
    private int width, height;
    private boolean started;

    private final ServiceConnection connection = new ServiceConnection() {
        @Override
        public void onServiceConnected(ComponentName name, IBinder binder) {
            desk = IDeskService.Stub.asInterface(binder);
            startDesk();
        }

        @Override
        public void onServiceDisconnected(ComponentName name) {
            desk = null;
            started = false;
            show("桌面服務中斷");
        }
    };

    private final Shizuku.UserServiceArgs serviceArgs = new Shizuku.UserServiceArgs(
            new ComponentName("com.eddlai.phonedesk", DeskService.class.getName()))
            .daemon(false)
            .processNameSuffix("desk")
            .version(2);

    private final Shizuku.OnRequestPermissionResultListener permissionListener = (code, result) -> {
        if (result == PackageManager.PERMISSION_GRANTED) bindDesk();
        else show("需要授權 Shizuku 權限");
    };

    private final Shizuku.OnBinderReceivedListener binderListener = this::connectShizuku;

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        surfaceView = new SurfaceView(this);
        surfaceView.getHolder().addCallback(this);
        surfaceView.setFocusable(true);
        surfaceView.setFocusableInTouchMode(true);
        surfaceView.setOnTouchListener((v, e) -> forward(e));
        surfaceView.setOnGenericMotionListener((v, e) -> forward(e));
        surfaceView.setOnHoverListener((v, e) -> forward(e));

        status = new TextView(this);
        status.setTextColor(Color.WHITE);
        status.setTextSize(18);
        status.setGravity(Gravity.CENTER);

        FrameLayout root = new FrameLayout(this);
        root.setBackgroundColor(Color.BLACK);
        root.addView(surfaceView);
        root.addView(status);
        setContentView(root);
        hideSystemBars();

        offerHomeShortcut();
        show("連線 Shizuku…");
        Shizuku.addRequestPermissionResultListener(permissionListener);
        Shizuku.addBinderReceivedListenerSticky(binderListener);
    }

    @Override
    protected void onDestroy() {
        Shizuku.removeBinderReceivedListener(binderListener);
        Shizuku.removeRequestPermissionResultListener(permissionListener);
        stopDesk();
        try {
            Shizuku.unbindUserService(serviceArgs, connection, true);
        } catch (Exception ignored) {
        }
        super.onDestroy();
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            hideSystemBars();
            surfaceView.requestFocus();
        }
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

    private void hideSystemBars() {
        WindowInsetsController c = getWindow().getInsetsController();
        if (c == null) return;
        c.hide(WindowInsets.Type.systemBars());
        c.setSystemBarsBehavior(WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
    }

    private void connectShizuku() {
        if (Shizuku.isPreV11()) {
            show("Shizuku 版本太舊");
        } else if (Shizuku.checkSelfPermission() == PackageManager.PERMISSION_GRANTED) {
            bindDesk();
        } else {
            Shizuku.requestPermission(0);
        }
    }

    private void bindDesk() {
        show("啟動桌面服務…");
        Shizuku.bindUserService(serviceArgs, connection);
    }

    @Override
    public void surfaceCreated(SurfaceHolder holder) {
    }

    @Override
    public void surfaceChanged(SurfaceHolder holder, int format, int w, int h) {
        surface = holder.getSurface();
        width = w;
        height = h;
        startDesk();
    }

    @Override
    public void surfaceDestroyed(SurfaceHolder holder) {
        // Keep the desk running in the background; its windows survive until the app is closed.
        if (desk != null && started) {
            try {
                desk.detach();
            } catch (RemoteException ignored) {
            }
        }
        started = false;
        surface = null;
    }

    private void startDesk() {
        if (desk == null || surface == null || width == 0) return;
        try {
            int id = desk.start(surface, width, height, DESK_DPI);
            started = true;
            status.setVisibility(TextView.GONE);
            Log.i(TAG, "desk on display " + id);
        } catch (Exception e) {
            show("建立虛擬螢幕失敗：" + e.getMessage());
        }
    }

    private void stopDesk() {
        if (desk == null) return;
        started = false;
        try {
            desk.stop();
        } catch (RemoteException ignored) {
        }
    }

    private boolean forward(MotionEvent e) {
        if (!started) return false;
        try {
            // The virtual display has the same size as the surface, so coordinates map 1:1.
            desk.injectMotion(e);
        } catch (RemoteException ignored) {
        }
        return true;
    }

    @Override
    public boolean dispatchKeyEvent(KeyEvent e) {
        if (!started) return super.dispatchKeyEvent(e);
        try {
            desk.injectKey(e);
        } catch (RemoteException ignored) {
        }
        return true;
    }

    private void show(String text) {
        status.setText(text);
        status.setVisibility(TextView.VISIBLE);
    }
}
