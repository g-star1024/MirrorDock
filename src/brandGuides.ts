// 厂商品牌知识库：引导非技术用户在这台手机上打开开发者选项与 USB 调试。
// 内容以「通用路径 + 品牌特有注意点」表达；机型与系统版本众多，菜单名称可能
// 不同，界面必须如实提示「以手机实际设置为准」，不得假装路径永远准确。

export type BrandGuide = {
  key: string;
  name: string;
  /// 用于从设备 label（厂商 + 型号）猜测品牌的关键词，按出现的品牌优先匹配。
  match: string[];
  openDeveloperOptions: string;
  usbDebugging: string;
  wireless: string;
  notes: string[];
};

export const brandGuides: BrandGuide[] = [
  {
    key: "pixel",
    name: "Pixel / 原生 Android",
    match: ["Pixel"],
    openDeveloperOptions: "设置 → 关于手机 → 连点「版本号」7 次，回到「设置 → 系统」即可看到「开发者选项」。",
    usbDebugging: "在「开发者选项」中打开「USB 调试」。",
    wireless: "「开发者选项」内打开「无线调试」，按 MirrorDock 的无线连接引导填写配对码。",
    notes: [
      "原生系统路径最稳定，一般不存在额外的厂商验证。",
    ],
  },
  {
    key: "samsung",
    name: "三星 Samsung",
    match: ["Samsung", "Galaxy", "SM-"],
    openDeveloperOptions: "设置 → 关于手机 → 软件信息 → 连点「编译编号」7 次，回到「设置」底部即可看到「开发者选项」。",
    usbDebugging: "在「开发者选项」中打开「USB 调试」。",
    wireless: "「开发者选项」内打开「无线调试」。",
    notes: [
      "One UI 各版本菜单名称略有差异；个别运营商会隐藏开发者选项相关入口。",
    ],
  },
  {
    key: "xiaomi",
    name: "小米 / Redmi",
    match: ["Xiaomi", "Redmi", "POCO", "小米", "红米"],
    openDeveloperOptions: "设置 → 我的设备 → 全部参数信息 → 连点「OS 版本」（旧系统为「MIUI 版本」）7 次，回到「设置 → 更多设置」即可看到「开发者选项」。",
    usbDebugging: "在「开发者选项」中打开「USB 调试」。若要在镜像窗口里**控制**手机，还需要打开「USB 调试（安全设置）」——它需要登录小米账号且手机插着 SIM 卡，开启时手机会提示等待，属正常现象。",
    wireless: "「开发者选项」内打开「无线调试」。",
    notes: [
      "「USB 调试」只允许看到画面；「USB 调试（安全设置）」才允许模拟点击与输入。两者都开才能完整控制。",
      "小米账号登录与 SIM 卡要求是系统限制，MirrorDock 无法绕过。",
    ],
  },
  {
    key: "oppo",
    name: "OPPO",
    match: ["OPPO", "realme", "真我"],
    openDeveloperOptions: "设置 → 关于本机 → 版本信息 → 连点「版本号」7 次，回到「设置 → 其他设置」即可看到「开发者选项」。",
    usbDebugging: "在「开发者选项」中打开「USB 调试」。ColorOS 连接时可能要求登录账号并输入验证码，按手机提示操作即可。",
    wireless: "「开发者选项」内打开「无线调试」。",
    notes: [
      "ColorOS 的账号验证与确认弹窗是系统行为，验证一次后同一台电脑通常不再重复。",
    ],
  },
  {
    key: "vivo",
    name: "vivo / iQOO",
    match: ["vivo", "iQOO"],
    openDeveloperOptions: "设置 → 系统管理 → 关于手机 → 连点「软件版本号」7 次，回到「设置 → 系统管理」即可看到「开发者选项」。",
    usbDebugging: "在「开发者选项」中打开「USB 调试」。OriginOS 连接时可能要求输入验证码或登录 vivo 账号，按手机提示操作即可。",
    wireless: "「开发者选项」内打开「无线调试」。",
    notes: [
      "子用户 / 访客模式下无法开启 USB 调试，请切换到机主账户操作。",
    ],
  },
  {
    key: "oneplus",
    name: "一加 OnePlus",
    match: ["OnePlus", "一加"],
    openDeveloperOptions: "设置 → 关于本机 → 版本信息 → 连点「版本号」7 次，回到「设置 → 系统设置」（旧系统为「其他设置」）即可看到「开发者选项」。",
    usbDebugging: "在「开发者选项」中打开「USB 调试」。较新版本的 ColorOS 机制与 OPPO 相同，可能要求登录账号并输入验证码。",
    wireless: "「开发者选项」内打开「无线调试」。",
    notes: [
      "一加近期系统与 OPPO 同源，遇到验证弹窗按 OPPO 的说明处理。",
    ],
  },
];

/// 从一组设备 label（厂商 + 型号）里猜最可能的品牌。返回 null 表示无法判断，
/// 界面不得把「猜不出」伪装成某个具体品牌。
export function detectBrand(labels: string[]): BrandGuide | null {
  const joined = labels.join(" ");
  for (const guide of brandGuides) {
    if (guide.match.some((keyword) => joined.includes(keyword))) {
      return guide;
    }
  }
  return null;
}
