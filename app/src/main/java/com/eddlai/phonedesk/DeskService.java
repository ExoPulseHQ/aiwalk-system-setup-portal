package com.eddlai.phonedesk;

import android.content.AttributionSource;
import android.content.Context;
import android.content.ContextWrapper;
import android.hardware.display.DisplayManager;
import android.hardware.display.VirtualDisplay;
import android.hardware.input.InputManager;
import android.os.IBinder;
import android.util.Log;
import android.view.Display;
import android.view.InputEvent;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.Surface;

import org.lsposed.hiddenapibypass.HiddenApiBypass;

import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.lang.reflect.Method;

/**
 * Shizuku user service running as uid 2000 (shell). Does only what needs shell rights:
 * the overlay display, mirroring it, injecting input, and toggling our accessibility service.
 * Android 17 does not let this process add windows (WMS rejects its unknown pid), so the
 * window itself lives in the app's accessibility service. Mirroring and the shell-context
 * trick follow scrcpy (Apache-2.0).
 */
public class DeskService extends IDeskService.Stub {
    private static final String TAG = "PhoneDesk";
    private static final String SHELL = "com.android.shell";
    private static final int TYPE_OVERLAY = 4; // Display.TYPE_OVERLAY (hidden)

    private final Context base;
    private final Context shell;

    private volatile int overlayId = -1;
    private VirtualDisplay mirror;
    private String savedAccel, savedUserRotation, savedA11y, savedA11yOn;

    private InputManager input;
    private Method injectMethod;
    private Method setDisplayIdMethod;

    // Shizuku passes a Context when this constructor exists (API 13+).
    public DeskService(Context context) {
        HiddenApiBypass.addHiddenApiExemptions("");
        base = context;
        shell = new ShellContext(context);
        log("service started, pid " + android.os.Process.myPid());
    }

    @Override
    public void destroy() {
        exitDisplay();
        System.exit(0);
    }

    @Override
    public synchronized int enterDisplay(int width, int height, int dpi) {
        if (overlayId >= 0) return overlayId;
        savedAccel = sh("settings get system accelerometer_rotation");
        savedUserRotation = sh("settings get system user_rotation");
        sh("settings put system accelerometer_rotation 0");
        sh("settings put system user_rotation 1");
        sh("settings put global force_desktop_mode_on_external_displays 1");
        // should_show_system_decorations: run a launcher/taskbar on it instead of mirroring display 0
        sh("settings put global overlay_display_devices " + width + "x" + height + "/" + dpi
                + ",should_show_system_decorations");
        int id = waitForOverlayDisplay();
        if (id < 0) {
            log("overlay display did not appear");
            exitDisplay();
            return -1;
        }
        overlayId = id;
        setImePolicyLocal(id);
        log("overlay display " + id + " " + width + "x" + height + "/" + dpi);
        return id;
    }

    @Override
    public synchronized boolean mirror(Surface surface, int width, int height) {
        releaseMirror();
        if (overlayId < 0 || surface == null) return false;
        try {
            Method m = DisplayManager.class.getMethod("createVirtualDisplay",
                    String.class, int.class, int.class, int.class, Surface.class);
            mirror = (VirtualDisplay) m.invoke(null, "PhoneDeskMirror", width, height, overlayId, surface);
            log("mirror " + width + "x" + height + " of display " + overlayId + " -> " + (mirror != null));
            return mirror != null;
        } catch (Exception e) {
            log("mirror failed: " + e + " / " + e.getCause());
            return false;
        }
    }

    @Override
    public synchronized void exitDisplay() {
        log("exitDisplay");
        releaseMirror();
        overlayId = -1;
        sh("settings put global overlay_display_devices none");
        if (savedAccel != null) {
            sh("settings put system accelerometer_rotation " + savedAccel);
            sh("settings put system user_rotation " + savedUserRotation);
            savedAccel = null;
        }
    }

    private void releaseMirror() {
        if (mirror != null) {
            mirror.release();
            mirror = null;
        }
    }

    @Override
    public void injectMotion(MotionEvent event) {
        inject(event);
    }

    @Override
    public void injectKey(KeyEvent event) {
        inject(event);
    }

    @Override
    public synchronized void setAccessibility(boolean enabled, String component) {
        log("setAccessibility " + enabled);
        if (enabled) {
            savedA11y = sh("settings get secure enabled_accessibility_services");
            savedA11yOn = sh("settings get secure accessibility_enabled");
            String list = savedA11y == null || savedA11y.isEmpty() || savedA11y.equals("null")
                    ? component : savedA11y.contains(component) ? savedA11y : savedA11y + ":" + component;
            // Sideloaded apps are "restricted": allow, or the system refuses to bind the service.
            sh("appops set com.eddlai.phonedesk ACCESS_RESTRICTED_SETTINGS allow");
            sh("settings put secure enabled_accessibility_services '" + list + "'");
            sh("settings put secure accessibility_enabled 1");
        } else if (savedA11y != null) {
            if (savedA11y.isEmpty() || savedA11y.equals("null")) {
                sh("settings delete secure enabled_accessibility_services");
            } else {
                sh("settings put secure enabled_accessibility_services '" + savedA11y + "'");
            }
            sh("settings put secure accessibility_enabled " + (savedA11yOn.equals("null") ? "0" : savedA11yOn));
            savedA11y = null;
        }
    }

    private int waitForOverlayDisplay() {
        DisplayManager dm = base.getSystemService(DisplayManager.class);
        for (int i = 0; i < 50; i++) {
            for (Display d : dm.getDisplays()) {
                if (displayType(d) == TYPE_OVERLAY) return d.getDisplayId();
            }
            try {
                Thread.sleep(100);
            } catch (InterruptedException e) {
                return -1;
            }
        }
        return -1;
    }

    private static int displayType(Display d) {
        try {
            return (int) Display.class.getMethod("getType").invoke(d);
        } catch (Exception e) {
            return -1;
        }
    }

    private void inject(InputEvent event) {
        int id = overlayId;
        if (id < 0) return;
        try {
            if (input == null) {
                input = (InputManager) shell.getSystemService(Context.INPUT_SERVICE);
                injectMethod = InputManager.class.getMethod("injectInputEvent", InputEvent.class, int.class);
                setDisplayIdMethod = InputEvent.class.getMethod("setDisplayId", int.class);
            }
            setDisplayIdMethod.invoke(event, id);
            injectMethod.invoke(input, event, 0); // INJECT_INPUT_EVENT_MODE_ASYNC
        } catch (Exception e) {
            log("inject failed: " + e + " / " + e.getCause());
        }
    }

    /** Show the soft keyboard on the desk instead of behind the overlay window. */
    private static void setImePolicyLocal(int id) {
        try {
            Class<?> sm = Class.forName("android.os.ServiceManager");
            IBinder binder = (IBinder) sm.getMethod("getService", String.class).invoke(null, "window");
            Object wm = Class.forName("android.view.IWindowManager$Stub")
                    .getMethod("asInterface", IBinder.class).invoke(null, binder);
            wm.getClass().getMethod("setDisplayImePolicy", int.class, int.class)
                    .invoke(wm, id, 0); // DISPLAY_IME_POLICY_LOCAL
        } catch (Exception e) {
            Log.e(TAG, "setDisplayImePolicy failed", e);
        }
    }

    /** Survives logcat rotation: /data/local/tmp is writable by shell and readable over adb. */
    private static synchronized void log(String msg) {
        Log.i(TAG, msg);
        try (java.io.FileWriter w = new java.io.FileWriter("/data/local/tmp/phonedesk.log", true)) {
            w.write(new java.text.SimpleDateFormat("MM-dd HH:mm:ss.SSS", java.util.Locale.US)
                    .format(new java.util.Date()) + " " + msg + "\n");
        } catch (Exception ignored) {
        }
    }

    /** Runs a shell command as uid 2000 and returns its trimmed stdout. */
    private static String sh(String cmd) {
        try {
            java.lang.Process p = Runtime.getRuntime().exec(new String[]{"sh", "-c", cmd});
            StringBuilder out = new StringBuilder();
            try (BufferedReader r = new BufferedReader(new InputStreamReader(p.getInputStream()))) {
                String line;
                while ((line = r.readLine()) != null) out.append(line);
            }
            p.waitFor();
            return out.toString().trim();
        } catch (Exception e) {
            Log.e(TAG, "sh " + cmd, e);
            return "";
        }
    }

    /** Makes system services believe the caller is the shell package, matching our uid. */
    private static final class ShellContext extends ContextWrapper {
        ShellContext(Context base) {
            super(base);
        }

        @Override
        public String getPackageName() {
            return SHELL;
        }

        @Override
        public String getOpPackageName() {
            return SHELL;
        }

        @Override
        public AttributionSource getAttributionSource() {
            return new AttributionSource.Builder(2000 /* shell uid */).setPackageName(SHELL).build();
        }

        @Override
        public Context getApplicationContext() {
            return this;
        }
    }
}
