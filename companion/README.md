# MirrorDock 伴侣 App（POC，C4-01 / C4-02）

Kotlin Android 应用，验证「桌面 ↔ 手机」不经 adb 的同网加密会话与 MediaProjection 捕获能力。

## 已实现（POC 范围，如实记录）

- **扫码配对（C4-01）**：CameraX + zxing 离线解码桌面端二维码；无相机可手动输入配对信息。
- **加密会话**：TLS 1.3（平台 SSLSocket），信任锚仅为二维码携带的服务器 SPKI SHA-256
  指纹（出带校验，防同网中间人）；配对码一次性、只经加密通道发送。
- **捕获 POC（C4-02）**：MediaProjection 同意/撤销状态机（NotRequested→Pending→Granted/Denied→Revoked）、
  mediaProjection 前台服务、ImageReader 帧计数 + 首帧 JPEG 样本上行、音频播放捕获能力探测（API 29+）。
- **POC 边界**：不做持续视频上行；统计消息在伴侣未连接时只进日志不发送。

## 构建

需要 Android SDK（platforms;android-34）与 JDK 17：

```bash
cd companion
gradle :app:assembleDebug   # 产物 app/build/outputs/apk/debug/app-debug.apk
```

CI：`.github/workflows/companion.yml` 在 push 到 main 时构建 debug APK 并上传 artifact。

## 与桌面端的协议（MDP1，POC）

- 二维码载荷：`MDP1|主机1,主机2|端口|一次性配对码|服务器SPKI SHA-256(hex)`
- 握手：TLS 建立后客户端首行发 `MDP1 <配对码>`，服务端回
  `{"type":"welcome","protocol":"MDP1"}` 或 `{"type":"rejected"}`。
- 后续为 JSON 行事件：`device_hello` / `capture_stats` / `bye`。
- 屏幕帧不落盘、不入日志（与桌面端同一铁律）；服务端只回传统计。
