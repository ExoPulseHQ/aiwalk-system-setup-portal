package com.eddlai.phonedesk;

import android.accessibilityservice.AccessibilityService;
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
 * hardware keys into it. Enabled by the shell service only while desktop mode is on.
 */
public class DeskOverlayService extends AccessibilityService {
    private static final String TAG = "PhoneDesk";
    static DeskOverlayService instance;

    private final Handler main = new Handler(Looper.getMainLooper());
    private FrameLayout window;
    private volatile boolean active;
    private int deskWidth, deskHeight;

    @Override
    protected void onServiceConnected() {
        instance = this;
        if (prefs().getBoolean("pending", false)) start();
    }

    @Override
    public boolean onUnbind(android.content.Intent intent) {
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
        p.edit().putBoolean("pending", false).apply();
        int ml = p.getInt("ml", 0), mt = p.getInt("mt", 0), mr = p.getInt("mr", 0), mb = p.getInt("mb", 0);
        deskWidth = p.getInt("w", 0) - ml - mr;
        deskHeight = p.getInt("h", 0) - mt - mb;
        int dpi = p.getInt("dpi", 240);
        DeskConnection.whenReady(() -> new Thread(() -> {
            try {
                int id = DeskConnection.get().enterDisplay(deskWidth, deskHeight, dpi);
                if (id < 0) {
                    exit();
                    return;
                }
                active = true;
                main.post(() -> showWindow(ml, mt, mr, mb));
            } catch (Exception e) {
                Log.e(TAG, "enter failed", e);
                exit();
            }
        }).start());
    }

    /** Back to phone mode. Safe to call more than once. */
    void exit() {
        active = false;
        main.post(() -> {
            if (window != null) {
                try {
                    getSystemService(WindowManager.class).removeView(window);
                } catch (Exception ignored) {
                }
                window = null;
            }
            MainActivity.finishHost();
        });
        new Thread(() -> {
            IDeskService desk = DeskConnection.get();
            if (desk == null) return;
            try {
                desk.exitDisplay();
                desk.setAccessibility(false, getPackageName() + "/" + DeskOverlayService.class.getName());
            } catch (Exception e) {
                Log.e(TAG, "exit failed", e);
            }
        }).start();
    }

    private void showWindow(int ml, int mt, int mr, int mb) {
        if (!active || window != null) return;
        window = new FrameLayout(this);
        window.setBackgroundColor(Color.BLACK);

        SurfaceView view = new SurfaceView(this);
        view.getHolder().addCallback(new SurfaceHolder.Callback() {
            @Override
            public void surfaceCreated(SurfaceHolder h) {
                call(d -> d.mirror(h.getSurface(), deskWidth, deskHeight));
            }

            @Override
            public void surfaceChanged(SurfaceHolder h, int f, int w, int hh) {
            }

            @Override
            public void surfaceDestroyed(SurfaceHolder h) {
                call(d -> d.mirror(null, 0, 0));
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
        back.setOnClickListener(v -> exit());
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

    private boolean forward(MotionEvent e) {
        MotionEvent copy = MotionEvent.obtain(e);
        call(d -> d.injectMotion(copy));
        return true;
    }

    /** Hardware keyboards (scrcpy, Bluetooth) type into the desk while it is shown. */
    @Override
    protected boolean onKeyEvent(KeyEvent event) {
        if (!active) return false;
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
