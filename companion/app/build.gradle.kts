plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.mirrordock.companion"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.mirrordock.companion"
        minSdk = 26
        targetSdk = 34
        versionCode = 2
        versionName = "0.1.1-poc"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    // 仅扫码解析核心（纯 Java，无相机/界面依赖）；相机流用系统 Camera2 手写最小预览。
    implementation("com.google.zxing:core:3.5.3")
    // CameraX 最小集：预览 + 图像分析（ML Kit 不用，离线可用）。
    val camerax = "1.3.4"
    implementation("androidx.camera:camera-core:$camerax")
    implementation("androidx.camera:camera-camera2:$camerax")
    implementation("androidx.camera:camera-lifecycle:$camerax")
    implementation("androidx.camera:camera-view:$camerax")
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.appcompat:appcompat:1.7.0")
}
