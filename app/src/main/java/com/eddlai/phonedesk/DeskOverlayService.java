package com.eddlai.phonedesk;

import android.accessibilityservice.AccessibilityService;
import android.content.Intent;
import android.content.SharedPreferences;
import android.graphics.Color;
import android.graphics.PixelFormat;
import android.graphics.drawable.GradientDrawable;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.view.WindowManager;
import android.view.accessibility.AccessibilityEvent;
import android.widget.FrameLayout;

/**
 * Draws the desk: a TYPE_ACCESSIBILITY_OVERLAY window (layer 31, above the overlay display's
 * own floating preview at 29) showing the mirrored overlay display, forwarding touch and
 * hardware keys into it.
 *
 * Safety, because this window covers the whole phone:
 * - the service only lives while a start was requested; any other bind disables it at once;
 * - a watchdog exits if the desk is not up within WATCHDOG_MS;
 * - holding volume up + volume down together always exits;
 * - every exit path removes the window before touching anything else.
 */
public class DeskOverlayService extends AccessibilityService {
    private static final String TAG = "PhoneDesk";
    private static final long WATCHDOG_MS = 8000;
    static DeskOverlayService instance;

    private final Handler main = new Handler(Looper.getMainLooper());
    private FrameLayout window;
    private volatile boolean active;
    private volatile boolean mirrored;
    private boolean volUp, volDown;
    private int deskWidth, deskHeight;

    private final Runnable watchdog = () -> {
        Log.i(TAG, "watchdog: mirrored=" + mirrored + " window=" + (window != null));
        if (!mirrored) {
            Log.w(TAG, "watchdog: desk not up in time, exiting");
            exit();
        }
    };

    @Override
    protected void onServiceConnected() {
        instance = this;
        SharedPreferences p = prefs();
        if (p.getBoolean("pending", false)) {
            start();
        } else {
            // Rebound by the system (process restart, stale setting): never linger.
            Log.w(TAG, "bound without a pending start, disabling");
            disableSelf();
        }
    }

    @Override
    public boolean onUnbind(Intent intent) {
        removeWindow();
        active = false;
        instance = null;
        return super.onUnbind(intent);
    }

    @Override
    public void onAccessibilityEvent(AccessibilityEvent event) {
    }

    @Override
    public void onInterrupt() {
    }

    private SharedPreferences prefs() {
        return getSharedPreferences("desk", MODE_PRIVATE);
    }

    /** Builds the desk from the parameters the activity stored. */
    void start() {
        if (active) return;
        SharedPreferences p = prefs();
        p.edit().putBoolean("pending", false).commit(); // commit: must survive an immediate kill
        int ml = p.getInt("ml", 0), mt = p.getInt("mt", 0), mr = p.getInt("mr", 0), mb = p.getInt("mb", 0);
        deskWidth = p.getInt("w", 0) - ml - mr;
        deskHeight = p.getInt("h", 0) - mt - mb;
        int dpi = p.getInt("dpi", 240);
        active = true;
        mirrored = false;
        main.postDelayed(watchdog, WATCHDOG_MS);
        DeskConnection.whenReady(() -> new Thread(() -> {
            try {
                int id = DeskConnection.get().enterDisplay(deskWidth, deskHeight, dpi);
                if (id < 0 || !active) {
                    exit();
                    return;
                }
                main.post(() -> showWindow(ml, mt, mr, mb));
            } catch (Exception e) {
                Log.e(TAG, "enter failed", e);
                exit();
            }
        }).start());
    }

    /** Back to phone mode. Safe to call more than once and from any thread. */
    void exit() {
        Log.i(TAG, "exit requested", new Throwable("caller"));
        active = false;
        main.removeCallbacks(watchdog);
        main.post(() -> {
            removeWindow();
            MainActivity.finishHost();
        });
        new Thread(() -> {
            IDeskService desk = DeskConnection.get();
            try {
                if (desk != null) desk.exitDisplay();
            } catch (Exception e) {
                Log.e(TAG, "exitDisplay failed", e);
            }
            main.post(this::disableSelf); // also clears our entry from enabled services
        }).start();
    }

    private void removeWindow() {
        if (window == null) return;
        try {
            getSystemService(WindowManager.class).removeView(window);
        } catch (Exception ignored) {
        }
        window = null;
    }

    private void showWindow(int ml, int mt, int mr, int mb) {
        if (!active || window != null) return;
        window = new FrameLayout(this);
        window.setBackgroundColor(Color.BLACK);

        SurfaceView view = new SurfaceView(this);
        view.getHolder().addCallback(new SurfaceHolder.Callback() {
            @Override
            public void surfaceCreated(SurfaceHolder h) {
                Log.i(TAG, "surfaceCreated");
                attachMirror(h);
            }

            @Override
            public void surfaceChanged(SurfaceHolder h, int f, int w, int hh) {
                Log.i(TAG, "surfaceChanged " + w + "x" + hh);
            }

            @Override
            public void surfaceDestroyed(SurfaceHolder h) {
                // The surface is recreated on relayout (e.g. rotation); surfaceCreated re-attaches.
                Log.i(TAG, "surfaceDestroyed");
            }
        });
        view.setOnTouchListener((v, e) -> forward(e));
        view.setOnGenericMotionListener((v, e) -> forward(e));
        FrameLayout.LayoutParams vp = new FrameLayout.LayoutParams(deskWidth, deskHeight);
        vp.setMargins(ml, mt, mr, mb);
        window.addView(view, vp);

        // Way back to phone mode, in the left margin strip.
        View back = new View(this);
        GradientDrawable dot = new GradientDrawable();
        dot.setShape(GradientDrawable.OVAL);
        dot.setColor(0x80FFFFFF);
        back.setBackground(dot);
        back.setContentDescription("切換回手機模式");
        back.setOnClickListener(v -> {
            Log.i(TAG, "exit dot tapped");
            exit();
        });
        int size = 56;
        FrameLayout.LayoutParams bp = new FrameLayout.LayoutParams(size, size, Gravity.START | Gravity.BOTTOM);
        bp.setMargins(Math.max(0, (ml - size) / 2), 0, 0, 40);
        window.addView(back, bp);

        WindowManager.LayoutParams lp = new WindowManager.LayoutParams(
                WindowManager.LayoutParams.MATCH_PARENT, WindowManager.LayoutParams.MATCH_PARENT,
                WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
                WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN
                        | WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS
                        | WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE,
                PixelFormat.OPAQUE);
        lp.layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS;
        lp.setFitInsetsTypes(0);
        lp.setTitle("PhoneDesk");
        try {
            getSystemService(WindowManager.class).addView(window, lp);
        } catch (Exception e) {
            Log.e(TAG, "addView failed", e);
            window = null;
            exit();
        }
    }

    /** Points the mirror at this surface; the desk counts as up once one attach succeeds. */
    private void attachMirror(SurfaceHolder h) {
        new Thread(() -> {
            IDeskService desk = DeskConnection.get();
            boolean ok = false;
            try {
                ok = desk != null && desk.mirror(h.getSurface(), deskWidth, deskHeight);
            } catch (Exception e) {
                Log.e(TAG, "mirror failed", e);
            }
            Log.i(TAG, "mirror attached: " + ok);
            if (ok) mirrored = true;
            else exit();
        }).start();
    }

    private boolean forward(MotionEvent e) {
        MotionEvent copy = MotionEvent.obtain(e);
        call(d -> d.injectMotion(copy));
        return true;
    }

    /** Hardware keys go to the desk; volume up + down together is the emergency exit. */
    @Override
    protected boolean onKeyEvent(KeyEvent event) {
        if (!active) return false;
        int code = event.getKeyCode();
        if (code == KeyEvent.KEYCODE_VOLUME_UP || code == KeyEvent.KEYCODE_VOLUME_DOWN) {
            boolean down = event.getAction() == KeyEvent.ACTION_DOWN;
            if (code == KeyEvent.KEYCODE_VOLUME_UP) volUp = down;
            else volDown = down;
            if (volUp && volDown) {
                Log.w(TAG, "emergency exit (volume up + down)");
                exit();
            }
            return false; // volume keys keep working normally
        }
        call(d -> d.injectKey(event));
        return true;
    }

    private interface Call {
        void run(IDeskService desk) throws Exception;
    }

    private static void call(Call c) {
        IDeskService desk = DeskConnection.get();
        if (desk == null) return;
        try {
            c.run(desk);
        } catch (Exception e) {
            Log.e(TAG, "desk call failed", e);
        }
    }
}
