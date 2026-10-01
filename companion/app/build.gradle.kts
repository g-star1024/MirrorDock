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
        // M4-5：伴侣端版本独立维护；0.2.0 = M4 常驻通道/状态通知/解除互信。
        versionCode = 11
        versionName = "0.2.0"
    }

    // 固定签名（X10-21）：CI 从 GitHub Secrets 注入密钥库与口令，保证跨版本
    // 覆盖安装（INSTALL_FAILED_UPDATE_INCOMPATIBLE 的根治）。本地无环境变量时
    // 回退默认 debug 密钥（仅本地调试用，与 CI 产物签名不同）。
    signingConfigs {
        val storePath = System.getenv("COMPANION_KEYSTORE")
        val storePass = System.getenv("COMPANION_STORE_PASSWORD")
        if (!storePath.isNullOrBlank() && !storePass.isNullOrBlank()) {
            create("ci") {
                storeFile = file(storePath)
                storePassword = storePass
                keyAlias = "mirrordock-companion"
                keyPassword = storePass
            }
        }
    }

    buildTypes {
        debug {
            signingConfigs.findByName("ci")?.let { signingConfig = it }
        }
        release {
            isMinifyEnabled = false
            signingConfigs.findByName("ci")?.let { signingConfig = it }
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
