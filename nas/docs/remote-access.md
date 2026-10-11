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

飞牛**系统自带** DDNS 和远程访问（见下方「飞牛自带功能」），但**没有自带的反向代理**——反代需要第三方应用（Lucky、NPM、Caddy）。目标是：`https://mirror.你的域名` → `http://NAS内网IP:8080`。

### 飞牛自带远程访问（系统设置 → 远程访问）

飞牛 fnOS 系统设置里有四种远程访问方式，按推荐排序：

| 方式 | 原理 | 配置成本 | 速度 | 适用场景 |
|---|---|---|---|---|
| **FN Connect**（推荐） | 飞牛自研中继+ P2P，生成 `fnos.net/你的ID` 专属链接 | 零（登录飞牛账号即可） | 中继有限速，P2P 直连无损 | 最省心，无需公网 IP/域名 |
| **DDNS + 证书** | 腾讯云/阿里云/华为云/花生壳/DuckDNS 域名解析到 NAS | 中（需买域名+配置） | 无损 | 有域名、要 HTTPS 硬解 |
| **公网 IP 直连** | 路由器端口转发到飞牛 8000(HTTP)/8001(HTTPS) | 低（需申请公网 IP） | 无损 | 有公网 IPv4/IPv6 |
| **P2P** | 飞牛 App 自动切换 IPv6 直连 | 零（FN Connect 下自动） | 无损 | 网络环境支持 IPv6 |

**配置路径**：飞牛桌面 → 系统设置 → 远程访问 → 选对应方式。

**关键结论**：飞牛自带功能**管「怎么连到 NAS」**，但**不管「怎么把域名映射到具体服务」**——后者才是反向代理的活。

#### FN Connect 访问 MirrorDock（用户实踩澄清，2026-10-11）

**`https://<你的FN ID>.fnos.net:8080` 这样访问不到 Docker 容器**——FN Connect 域名
只代理到飞牛管理面板（443），域名后加自定义端口是飞牛官方明确不支持的路径
（身份认证必须经过面板/APP 跳转）。两条正路：

**路径 1：管理面板 → Docker 入口（零配置）**
1. 浏览器打开 `https://<你的FN ID>.fnos.net` 并**先登录飞牛管理面板**——不登录
   直接访问应用链接会报「FN Connect 暂无权限访问该服务」；
2. 进入 **Docker 管理器**，找到 `mirrordock-nas` 容器；
3. 点容器后的 **🔗 链接图标 → 点端口名（8080）**，飞牛在新窗口打开一个自动
   生成的三级域名（形如 `https://<随机串>.<你的ID>.5ddd.com`）；
4. 看到 MirrorDock 自己的登录页，输入访问密码即可。

注意：同一浏览器保持管理面板登录态；自动域名不可自定义；手机飞牛 APP 内从
Docker 入口打开（不支持丢给第三方浏览器）。

**路径 2：FN Connect 端口穿透规则（固定地址）**
1. 系统设置 → 远程访问 → FN Connect，新增**端口转发/穿透规则**；
2. 内网目标地址 = NAS 局域网 IP，内网端口 = `8080`，穿透类型 = **TCP+HTTP**；
3. 保存后飞牛分配固定域名，以后直接访问，不必钻 Docker 管理器。

**FN Connect 的代价**：中继模式流量过飞牛服务器，看镜像/录像吃中转流量且
延迟高——远程看实时镜像画质锁 **Smooth** 档。有 IPv6/公网 IP 时，DDNS + Lucky
反代（见下）才是长期正解。

### 反向代理（第三方应用，二选一）

**Lucky（推荐，飞牛应用中心直接装）**：
1. 应用中心搜「Lucky」安装，浏览器访问 `http://NAS_IP:16601`（默认账号密码 `666`，登录后改密码）。
2. **DDNS**：侧栏「动态域名」→ 添加任务 → 选你的域名托管商（Cloudflare/阿里云等）→ 填 Token → 添加同步记录（主域名 + 泛解析 `*.域名.com`）。
3. **SSL 证书**：侧栏「SSL/TLS 证书」→ 添加证书 → ACME → 选托管商 → 填 Token → 域名列表同 DDNS → 等几分钟自动签发。
4. **反向代理**：侧栏「Web 服务」→ 添加 Web 服务规则：
   - 监听类型：按你的网络选（IPv4/IPv6）；
   - 监听端口：默认 16666（可改，外网访问用这个端口）；
   - **TLS 必须开启**（HTTPS 硬解的前提）；
   - 添加子规则：服务类型「反向代理」→ 前端地址 `mirror.你的域名.com` → 后端地址 `http://192.168.1.45:8080`（你的 NAS 内网 IP + MirrorDock 端口）。
5. **路由器端口转发**：把 Lucky 的监听端口（如 16666）映射到 NAS。
6. 外网访问 `https://mirror.你的域名.com:16666`。

**Nginx Proxy Manager（NPM，Docker 部署）**：
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

### 关键要点

- **HTTPS 是安全上下文**，WebCodecs 自动解锁，镜像走硬件解码，最省 NAS CPU。
- **WebSocket 必须转发**（上面 nginx 配置），否则镜像卡住。
- **Basic Auth 只是网关门卫**，**仍然建议开启方案 C**——网关配置失误（比如
  某条 location 漏挂了 Access List）时应用层还有一道。
- **登录限速按来源 IP 计数**：反代后所有请求都来自反代 IP，限速粒度会变粗。
  如需真实 IP，在反代传 `X-Forwarded-For`（当前版本按连接地址计数，够用）。
- **飞牛自带 DDNS + Lucky 反代是绝配**：飞牛管域名解析，Lucky 管 SSL + 反代，
  两者不冲突。

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
