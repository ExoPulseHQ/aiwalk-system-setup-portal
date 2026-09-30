package com.eddlai.phonedesk;

import android.view.Surface;
import android.view.MotionEvent;
import android.view.KeyEvent;

// Runs as the shell user inside a Shizuku user service.
interface IDeskService {
    // Shizuku calls this transaction code to tear the service down.
    void destroy() = 16777114;

    // Creates a trusted virtual display rendering into surface; returns its display id.
    int start(in Surface surface, int width, int height, int dpi) = 1;
    void stop() = 2;
    oneway void injectMotion(in MotionEvent event) = 3;
    oneway void injectKey(in KeyEvent event) = 4;
}
