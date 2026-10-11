# MirrorDock NAS 版 · 远程访问部署指南

NAS 版默认只应暴露在局域网。要从外面（手机流量、公司网络、出差酒店）访问，
有且只有三条正路，按推荐顺序排列。**不要**直接把 8080 端口映射到公网——
即使设了访问密码，明文 HTTP 也会让密码和画面在链路上裸奔。

| 方案 | 安全性 | 配置成本 | 访问端要求 | WebCodecs 硬解 |
|---|---|---|---|---|
| A. Tailscale / 飞牛 VPN 组网 | ★★★（端到端加密，零暴露） | 低 | 每台设备装客户端 | 需配合 HTTPS 或用 WASM 软解 |
| B. 反代 + HTTPS + Basic Auth | ★★☆（HTTPS 加密 + 网关密码） | 中 | 任意浏览器 | ✅ 自动解锁 |
| C. 应用内访问密码（X10-119） | ★★☆（应用层密码 + 会话 Token） | 零（页面里设置） | 任意浏览器 | 需配合 HTTPS |

三条路**不互斥**：A/B 管「链路加密与网络边界」，C 管「应用层最后一道门」。
挂公网时建议 B+C 或 A+C 组合。

---

## 方案 A：Tailscale 组网（首选，最省心）

把 NAS 和你的手机/电脑拉进同一个虚拟内网，外面的人根本看不见这台 NAS。

1. NAS（飞牛）上安装 Tailscale：
   - 飞牛应用中心直接搜「Tailscale」安装，或
   - SSH 到 NAS 执行官方脚本：`curl -fsSL https://tailscale.com/install.sh | sh`
2. `sudo tailscale up` 登录（Google/Microsoft/邮箱均可），记下分配到的
   100.x.x.x 地址。
3. 手机/电脑装 Tailscale 客户端并登录同一账号（或用 ACL 共享节点）。
4. 浏览器访问 `http://100.x.x.x:8080`——和在家里局域网一模一样。

要点：
- 流量走 WireGuard 端到端加密，**不需要在路由器上开任何端口**。
- 地址是 HTTP（非 HTTPS），所以 WebCodecs 不可用，镜像自动回落 WASM 软解
  （X10-118 已内置）。画面照常，NAS CPU 占用略高。
- 想要硬解：给 Tailscale 节点开 HTTPS（`tailscale cert`，需要启用 MagicDNS
  和 HTTPS 证书功能），访问 `https://<节点名>.<tailnet>.ts.net:8080`
  （反代到本地 8080）。
- 建议同时开启**方案 C 访问密码**：Tailscale 账号被盗 = 门户大开，应用层
  再挡一道。

## 方案 B：反向代理 + HTTPS + Basic Auth（适合挂域名）

飞牛自带「远程访问/DDNS + 反向代理」，或用 Nginx Proxy Manager（NPM）、
Caddy、群晖反代均可。目标是：`https://mirror.你的域名` → `http://NAS内网IP:8080`。

以 Nginx Proxy Manager 为例：

1. **Proxy Host**：域名 `mirror.example.com` → Forward `http://<NAS_IP>:8080`。
2. **SSL**：申请 Let's Encrypt 证书，开 Force SSL + HTTP/2。
3. **Custom locations / Advanced** 里确认 WebSocket 转发（NPM 默认已开
   `Upgrade`/`Connection` 头；手写 nginx 的话补：
   ```nginx
   proxy_set_header Upgrade $http_upgrade;
   proxy_set_header Connection "upgrade";
   ```
   缺这两行的症状：页面能开、镜像一直「连接中」）。
4. **Access List**：新建 Basic Auth（用户名+密码），绑到 Proxy Host。
   这是网关层密码，先于 MirrorDock 自己的登录页生效。
5. 上传大文件（APK/视频最高 2GB）需要调大反代 body 上限：
   NPM 默认够大；手写 nginx 加 `client_max_body_size 2g;`。

要点：
- HTTPS 是**安全上下文**，WebCodecs 自动解锁，镜像走硬件解码，最省 NAS CPU。
- Basic Auth 只是网关门卫，**仍然建议开启方案 C**——网关配置失误（比如
  某条 location 漏挂了 Access List）时应用层还有一道。
- 登录限速按来源 IP 计数：反代后所有请求都来自反代 IP，限速粒度会变粗。
  如需真实 IP，在反代传 `X-Forwarded-For`（当前版本按连接地址计数，够用）。

## 方案 C：应用内访问密码（X10-119，已内置）

不依赖任何外部组件，在页面上就能开关：

1. 打开 MirrorDock 页面 → 底部「设置」卡片 → 「访问密码」区。
2. 输入新密码（≥6 位）→「更新密码」。立即生效，**所有会话作废**，
   其他设备需重新登录。
3. 之后打开页面会先看到登录遮罩；登录成功后会话保持 12 小时（滑动续期）。
4. 清除密码：填当前密码、新密码留空 →「更新密码」。

部署者兜底（compose）：

```yaml
environment:
  # 显式设置后以 env 为准，Web 修改无效（防网页访客改密码）
  - MIRRORDOCK_ACCESS_PASSWORD=你的强密码
```

安全细节（实现见 `server/src/auth.rs`）：
- 密码只存 PBKDF2-HMAC-SHA256 哈希（600k 迭代、随机盐、恒定时间比较），
  settings.json 泄露不等于密码泄露。
- 会话 Token 128 位随机，内存保存，12 小时空闲过期；改密码/清除密码立即
  作废旧会话。
- 登录按来源 IP 限速：1 分钟错 5 次锁 1 分钟（429）。
- Token 传递：HTTP 走 `Authorization: Bearer`；WebSocket 与 `<img>`/下载
  直链走 `?token=`（浏览器限制，无法自定义头）。

**方案 C 的边界**：它管「谁能用」，不管「链路加密」。HTTP 下密码哈希不出
本机，但登录后的 Cookie/Token 与画面数据是明文——所以挂公网必须叠 A 或 B。

---

## 常见问题

**Q：远程看镜像卡/糊？**
远程场景在「设置」里把画质锁到「流畅（1024p · 2Mbps）」。瓶颈几乎一定是
家里上行带宽，不是 NAS。

**Q：忘记访问密码？**
SSH 到 NAS，删掉数据目录里的 `settings.json` 中的 `access_password` 字段
（或整个文件），重启容器；或在 compose 设 `MIRRORDOCK_ACCESS_PASSWORD`
强制覆盖。

**Q：Tailscale 下镜像不出画面？**
`http://100.x:8080` 是非安全上下文 → 自动 WASM 软解，首次解码要加载
h264bsd.wasm（121KB），慢一两秒正常。仍不出画面看页面「实时镜像」卡片的
错误提示（X10-118 会把设备端 server 的真实输出带出来）。

**Q：多设备访问会互相看到吗？**
会。当前是单实例单密码模型，所有登录者能力相同（都能看画面、操控、删文件）。
不要把密码发给不信任的人；要给人演示，建议临时开 Tailscale 共享节点。
