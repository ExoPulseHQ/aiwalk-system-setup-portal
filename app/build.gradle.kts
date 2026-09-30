plugins { id("com.android.application") }

android {
    namespace = "com.eddlai.phonedesk"
    compileSdk = 36
    defaultConfig {
        applicationId = "com.eddlai.phonedesk"
        minSdk = 34
        targetSdk = 36
        versionCode = 1
        versionName = "0.1"
    }
    buildFeatures { aidl = true }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation("dev.rikka.shizuku:api:13.1.5")
    implementation("dev.rikka.shizuku:provider:13.1.5")
    implementation("org.lsposed.hiddenapibypass:hiddenapibypass:6.1")
}
