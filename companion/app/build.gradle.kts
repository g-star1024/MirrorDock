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
        // 0.3.1 = X10-76 设计令牌层（dimens.xml，113 处硬编码 dp 收敛）
        //        + 暗色主题（values-night，此前完全缺失）
        //        + 4 处 WCAG AA 对比度硬失败修复（实测 2.15-2.60 → 4.7-6.0）
        //        + 触控目标 40/42/44/46dp → 48dp（审计实测 11 处不达标）
        //        + 字号 12 级 → 5 级 + 首次显示版本号（此前 App 里一处都没有）
        // 0.3.0 = X10-73 三 Tab 设备中心重构 + 多设备列表/分组 + 连接质量面板
        //        + 桌面 pinned shortcut（协议对旧版桌面端双向兼容）。
        // 0.2.5 = X10-69 快捷回复（notification_reply 下行 + RemoteInput 填充）。
        // 0.2.4 = X10-66 通知镜像一期 + X10-64B 默认端口回退；0.2.3 = 常驻会话保活修复。
        versionCode = 18
        versionName = "0.3.1"
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
