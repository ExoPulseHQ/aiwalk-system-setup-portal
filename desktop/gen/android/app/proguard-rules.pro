# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile

# MainActivity.Files is called by name from the page (window.aiwalkFiles); the release build must keep its methods
-keepclassmembers class com.aiwalk.setup.MainActivity$Files {
   @android.webkit.JavascriptInterface public *;
}

# KeyUsb is called only from Rust through JNI (fidokey.rs), which R8 cannot see: without this the release APK keeps
# the class but drops devices/open/send/recv, and the app never finds a plugged-in key (0.3.31).
-keep class com.aiwalk.setup.KeyUsb { *; }
