package com.eddlai.phonedesk;

import android.view.Surface;
import android.view.MotionEvent;
import android.view.KeyEvent;

// Runs as the shell user inside a Shizuku user service: only the things that need shell rights.
interface IDeskService {
    // Shizuku calls this transaction code to tear the service down.
    void destroy() = 16777114;

    // Creates the overlay display the desktop runs on (Pixel's desktop mode accepts overlay
    // displays, not app virtual displays) and locks the phone to landscape. Returns its id or -1.
    int enterDisplay(int width, int height, int dpi) = 1;

    // Mirrors the overlay display into surface (drawn by our accessibility overlay window).
    // Returns false if the mirror could not be created.
    boolean mirror(in Surface surface, int width, int height) = 2;

    // Removes the overlay display and restores rotation.
    void exitDisplay() = 3;

    oneway void injectMotion(in MotionEvent event) = 4;
    oneway void injectKey(in KeyEvent event) = 5;

    // Turns our accessibility service on/off without the settings UI (sideloaded apps are
    // blocked there by "restricted settings").
    void setAccessibility(boolean enabled, String component) = 6;
}
