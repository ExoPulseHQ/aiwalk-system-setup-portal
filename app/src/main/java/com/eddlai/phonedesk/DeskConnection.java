package com.eddlai.phonedesk;

import android.content.ComponentName;
import android.content.ServiceConnection;
import android.os.IBinder;

import java.util.ArrayList;
import java.util.List;

import rikka.shizuku.Shizuku;

/** One Shizuku binding to the shell-side DeskService, shared by the activity and overlay. */
final class DeskConnection {
    private static IDeskService service;
    private static boolean binding;
    private static final List<Runnable> waiting = new ArrayList<>();

    private static final Shizuku.UserServiceArgs ARGS = new Shizuku.UserServiceArgs(
            new ComponentName("com.eddlai.phonedesk", DeskService.class.getName()))
            .daemon(true)
            .processNameSuffix("desk")
            .version(14);

    private static final ServiceConnection CONNECTION = new ServiceConnection() {
        @Override
        public void onServiceConnected(ComponentName name, IBinder binder) {
            List<Runnable> ready;
            synchronized (DeskConnection.class) {
                service = IDeskService.Stub.asInterface(binder);
                binding = false;
                ready = new ArrayList<>(waiting);
                waiting.clear();
            }
            for (Runnable r : ready) r.run();
        }

        @Override
        public void onServiceDisconnected(ComponentName name) {
            synchronized (DeskConnection.class) {
                service = null;
            }
        }
    };

    static synchronized IDeskService get() {
        return service;
    }

    /** Runs then once the service is bound (immediately if it already is). Needs Shizuku permission. */
    static void whenReady(Runnable then) {
        synchronized (DeskConnection.class) {
            if (service == null) {
                waiting.add(then);
                if (!binding) {
                    binding = true;
                    Shizuku.bindUserService(ARGS, CONNECTION);
                }
                return;
            }
        }
        then.run();
    }

    private DeskConnection() {
    }
}
