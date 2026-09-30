package com.eddlai.phonedesk;

import android.content.AttributionSource;
import android.content.Context;
import android.content.ContextWrapper;
import android.hardware.display.DisplayManager;
import android.hardware.display.VirtualDisplay;
import android.hardware.input.InputManager;
import android.os.IBinder;
import android.util.Log;
import android.view.InputEvent;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.Surface;

import org.lsposed.hiddenapibypass.HiddenApiBypass;

import java.lang.reflect.Constructor;
import java.lang.reflect.Method;

/**
 * Shizuku user service: runs as uid 2000 (shell), which is allowed to create trusted
 * virtual displays and inject input. Display flags and the shell-context trick follow
 * scrcpy's NewDisplayCapture / FakeContext (Apache-2.0).
 */
public class DeskService extends IDeskService.Stub {
    private static final String TAG = "PhoneDesk";
    private static final String SHELL = "com.android.shell";

    private static final int FLAG_PUBLIC = DisplayManager.VIRTUAL_DISPLAY_FLAG_PUBLIC;
    private static final int FLAG_PRESENTATION = DisplayManager.VIRTUAL_DISPLAY_FLAG_PRESENTATION;
    private static final int FLAG_OWN_CONTENT_ONLY = DisplayManager.VIRTUAL_DISPLAY_FLAG_OWN_CONTENT_ONLY;
    private static final int FLAG_SUPPORTS_TOUCH = 1 << 6;
    private static final int FLAG_ROTATES_WITH_CONTENT = 1 << 7;
    private static final int FLAG_DESTROY_CONTENT_ON_REMOVAL = 1 << 8;
    private static final int FLAG_SHOULD_SHOW_SYSTEM_DECORATIONS = 1 << 9;
    private static final int FLAG_TRUSTED = 1 << 10;
    private static final int FLAG_OWN_DISPLAY_GROUP = 1 << 11;
    private static final int FLAG_ALWAYS_UNLOCKED = 1 << 12;
    private static final int FLAG_TOUCH_FEEDBACK_DISABLED = 1 << 13;
    private static final int FLAG_OWN_FOCUS = 1 << 14;
    private static final int FLAG_DEVICE_DISPLAY_GROUP = 1 << 15;

    private final Context shell;
    private VirtualDisplay display;
    private int displayId = -1;
    private InputManager input;
    private Method injectMethod;
    private Method setDisplayIdMethod;

    // Shizuku passes a Context when this constructor exists (API 13+).
    public DeskService(Context context) {
        HiddenApiBypass.addHiddenApiExemptions("");
        shell = new ShellContext(context);
    }

    @Override
    public void destroy() {
        stop();
        System.exit(0);
    }

    @Override
    public synchronized int start(Surface surface, int width, int height, int dpi) {
        if (display != null) {
            // Keep the desk (and its windows) alive across activity pauses: only swap the surface.
            display.resize(width, height, dpi);
            display.setSurface(surface);
            return displayId;
        }
        try {
            int flags = FLAG_PUBLIC | FLAG_PRESENTATION | FLAG_OWN_CONTENT_ONLY | FLAG_SUPPORTS_TOUCH
                    | FLAG_ROTATES_WITH_CONTENT | FLAG_DESTROY_CONTENT_ON_REMOVAL
                    | FLAG_SHOULD_SHOW_SYSTEM_DECORATIONS | FLAG_TRUSTED | FLAG_OWN_DISPLAY_GROUP
                    | FLAG_ALWAYS_UNLOCKED | FLAG_TOUCH_FEEDBACK_DISABLED | FLAG_OWN_FOCUS
                    | FLAG_DEVICE_DISPLAY_GROUP;
            Constructor<DisplayManager> ctor = DisplayManager.class.getDeclaredConstructor(Context.class);
            ctor.setAccessible(true);
            DisplayManager dm = ctor.newInstance(shell);
            display = dm.createVirtualDisplay("PhoneDesk", width, height, dpi, surface, flags);
            displayId = display.getDisplay().getDisplayId();
            Log.i(TAG, "virtual display " + width + "x" + height + "/" + dpi + " id=" + displayId);
            configureDisplay(displayId);
            return displayId;
        } catch (Exception e) {
            Log.e(TAG, "createVirtualDisplay failed", e);
            throw new IllegalStateException(e.toString());
        }
    }

    /**
     * Shell's "desktop supported" check rejects virtual displays, so make the display's root
     * task area freeform directly: apps then open as movable, stackable windows.
     */
    private static void configureDisplay(int id) {
        try {
            Class<?> sm = Class.forName("android.os.ServiceManager");
            IBinder binder = (IBinder) sm.getMethod("getService", String.class).invoke(null, "window");
            Object wm = Class.forName("android.view.IWindowManager$Stub")
                    .getMethod("asInterface", IBinder.class).invoke(null, binder);
            wm.getClass().getMethod("setWindowingMode", int.class, int.class)
                    .invoke(wm, id, 5); // WINDOWING_MODE_FREEFORM
            wm.getClass().getMethod("setDisplayImePolicy", int.class, int.class)
                    .invoke(wm, id, 0); // DISPLAY_IME_POLICY_LOCAL: keyboard shows on the desk
            Log.i(TAG, "display " + id + " set to freeform");
        } catch (Exception e) {
            Log.e(TAG, "configureDisplay failed", e);
        }
    }

    @Override
    public synchronized void detach() {
        if (display != null) display.setSurface(null);
    }

    @Override
    public synchronized void stop() {
        if (display != null) {
            display.release();
            display = null;
            displayId = -1;
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

    private void inject(InputEvent event) {
        int id = displayId;
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
            Log.e(TAG, "inject failed", e);
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
