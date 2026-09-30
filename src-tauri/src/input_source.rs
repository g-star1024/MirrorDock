//! 宿主（macOS）输入源自动切换（X10-39）。
//!
//! 背景：UHID 物理键盘语义下，按键以原始键码直送手机、不经过 Mac 输入法。
//! 第三方输入法（如微信输入法）激活时会把按键事件消费进自己的组字缓冲，
//! scrcpy 窗口收不到任何原始键，用户感知为「打字不生效」（X10-38 真机定案）。
//!
//! 方案：镜像会话开始时，若当前输入源是第三方输入法，临时切换到系统自带的
//! ABC 布局（未启用则先启用；X10-41 定案只有 ABC 能让镜像输入可用，拼音等
//! 输入法模式一律不行）；最后一个会话结束（或应用退出）时恢复用户原来的输入
//! 源。切换动作通过 Carbon 的 TIS（Text Input Source Services）API 完成，
//! 无弹窗、无辅助功能权限要求。
//!
//! 平台边界：只在 macOS 实现；Windows/Linux 编译为空操作（调用方会得到
//! `None`/`false`，自然跳过一切逻辑）。

/// ABC 布局的 InputSourceID（新版 macOS 上布局的 bundle id 统一收敛成
/// `com.apple.keyboardlayout.all`，只能靠 InputSourceID 区分，真机实测）。
pub(crate) const ABC_SOURCE_ID: &str = "com.apple.keylayout.ABC";

/// 会话开始时的纯决策：是否需要尝试切换输入源。
///
/// `already_managed` 表示本次应用运行期间已经切过一次（有备份）——输入源是
/// 宿主全局的，多台并发设备共享同一次切换，不必重复。
/// X10-41 真机定案：**只有 ABC 布局能让镜像输入可用**（第三方输入法与拼音
/// 等输入法模式都会吞键/组字），因此「当前不是 ABC」就值得切。
/// 返回 `true` 只代表「值得尝试」；实际切换可能失败（系统拒绝），由调用方兜底。
pub(crate) fn should_attempt_switch(current: Option<&str>, already_managed: bool) -> bool {
    if already_managed {
        return false;
    }
    current.is_some_and(|id| id != ABC_SOURCE_ID)
}

/// 会话结束时的纯决策：是否需要恢复用户原来的输入源。
///
/// `running_processes` 是当前仍在运行的镜像会话数量；只要还有一台在跑，
/// 就不能把输入法切回去（用户可能正在往另一台手机打字）。
pub(crate) fn should_attempt_restore(running_processes: usize, has_backup: bool) -> bool {
    has_backup && running_processes == 0
}

// ---------------------------------------------------------------------------
// macOS 实现：Carbon TIS FFI
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod imp {
    use super::ABC_SOURCE_ID;
    use std::ffi::{c_char, c_int, c_long, c_void, CStr};
    use std::sync::atomic::{AtomicBool, Ordering};

    /// ABC 是否是本次运行中由我们临时启用的（X10-41）：恢复时据此关回去，
    /// 不动用户原本的启用状态。进程崩溃残留的代价是输入法菜单多一个 ABC——无害。
    static ABC_ENABLED_BY_US: AtomicBool = AtomicBool::new(false);

    // Carbon/HIToolbox 的 TIS API。TISInputSourceRef 本质是 CFTypeRef（*mut c_void）。
    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        fn TISCopyCurrentKeyboardInputSource() -> *mut c_void;
        fn TISGetInputSourceProperty(source: *mut c_void, key: *const c_void) -> *mut c_void;
        fn TISSelectInputSource(source: *mut c_void) -> c_int; // OSStatus，0 = 成功
        fn TISEnableInputSource(source: *mut c_void) -> c_int;
        fn TISDisableInputSource(source: *mut c_void) -> c_int;
        fn TISCreateInputSourceList(
            properties: *const c_void,
            include_all_installed: bool,
        ) -> *mut c_void; // CFArrayRef（调用方持有，需释放）
        static kTISPropertyBundleID: *const c_void; // CFStringRef 属性键，取地址使用
        static kTISPropertyInputSourceID: *const c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringGetCString(
            string: *const c_void,
            buffer: *mut c_char,
            buffer_size: c_long,
            encoding: u32,
        ) -> bool;
        fn CFArrayGetCount(array: *const c_void) -> c_long;
        fn CFArrayGetValueAtIndex(array: *const c_void, index: c_long) -> *const c_void;
        fn CFRelease(value: *const c_void);
    }

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    /// `kTISPropertyBundleID` 的正确取值。它是一个**导出的数据符号**，槽位里
    /// 存的是 CFStringRef——属性 key 必须传「槽位里的值」。X10-40 教训：曾用
    /// `addr_of!` 把槽位地址当 key 传给 TIS，Carbon 拿垃圾指针当 CFString 解引用，
    /// `CFEqual → objc_msgSend → SIGSEGV`，开始镜像即崩（崩溃报告实锤）。
    fn tis_property_bundle_id_key() -> *const c_void {
        // extern static 的按值读取必须落在 unsafe 里；load 一次缓存于调用栈。
        unsafe { kTISPropertyBundleID }
    }

    /// `kTISPropertyInputSourceID` 的取值，语义同上（X10-41）。
    fn tis_property_input_source_id_key() -> *const c_void {
        unsafe { kTISPropertyInputSourceID }
    }

    fn cfstring_to_string(value: *const c_void) -> Option<String> {
        if value.is_null() {
            return None;
        }
        let mut buffer = [0 as c_char; 256];
        let ok = unsafe {
            CFStringGetCString(value, buffer.as_mut_ptr(), 256, K_CF_STRING_ENCODING_UTF8)
        };
        if !ok {
            return None;
        }
        Some(unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy().into_owned())
    }

    /// 输入源的「身份串」：优先 InputSourceID，读不到再退 BundleID。
    ///
    /// 为什么要两者：新版 macOS 把键盘布局的 BundleID 统一收敛成
    /// `com.apple.keyboardlayout.all`（真机实测，布局条目全部同名），只有
    /// InputSourceID（`com.apple.keylayout.ABC` 等）能区分具体布局；而第三方
    /// 输入法只有 BundleID 可用。备份/恢复/切换目标统一用这个身份串。
    fn source_identity(source: *mut c_void) -> Option<String> {
        unsafe {
            let by_id = cfstring_to_string(TISGetInputSourceProperty(
                source,
                tis_property_input_source_id_key(),
            ));
            by_id.or_else(|| {
                cfstring_to_string(TISGetInputSourceProperty(
                    source,
                    tis_property_bundle_id_key(),
                ))
            })
        }
    }

    /// 当前键盘输入源的身份串。第三方输入法（输入模式）激活时返回它的
    /// BundleID（如微信输入法）——正是我们要识别并临时切换掉的对象。
    pub fn current_bundle_id() -> Option<String> {
        unsafe {
            let source = TISCopyCurrentKeyboardInputSource();
            if source.is_null() {
                return None;
            }
            let result = source_identity(source);
            CFRelease(source as *const c_void);
            result
        }
    }

    /// 切换到 ABC 布局（X10-41 真机定案：**只有 ABC 能让镜像输入可用**）。
    ///
    /// X10-39 的三级兜底在只启用了系统拼音的 Mac 上会落到拼音——实测拼音模式
    /// 下镜像打字同样不可用（用户真机反馈），因此本版强制选 ABC：
    /// ① ABC 已启用 → 直接选中；
    /// ② ABC 未启用 → 从全部已安装列表找到它，先 `TISEnableInputSource` 再
    ///    选中（禁用状态的输入源无法直接选中），并记下「是我们启用的」，恢复
    ///    时还原这一改动；
    /// ③ 实在找不到 ABC（极罕见）→ 退回兜底：其它 Apple 布局 → Apple 输入法。
    pub fn switch_to_system_ascii() -> bool {
        unsafe {
            // ① 已启用的输入源里找 ABC。
            let enabled = TISCreateInputSourceList(std::ptr::null(), false);
            if !enabled.is_null() {
                let count = CFArrayGetCount(enabled);
                for index in 0..count {
                    let item = CFArrayGetValueAtIndex(enabled, index);
                    if item.is_null() {
                        continue;
                    }
                    if source_identity(item as *mut c_void).as_deref() == Some(ABC_SOURCE_ID) {
                        let ok = TISSelectInputSource(item as *mut c_void) == 0;
                        CFRelease(enabled);
                        return ok;
                    }
                }
                CFRelease(enabled);
            }

            // ② ABC 未启用：启用它再选中（先记「是我们启用的」，恢复时关回去）。
            let all = TISCreateInputSourceList(std::ptr::null(), true);
            if !all.is_null() {
                let count = CFArrayGetCount(all);
                for index in 0..count {
                    let item = CFArrayGetValueAtIndex(all, index);
                    if item.is_null() {
                        continue;
                    }
                    if source_identity(item as *mut c_void).as_deref() == Some(ABC_SOURCE_ID) {
                        let switched = TISEnableInputSource(item as *mut c_void) == 0
                            && TISSelectInputSource(item as *mut c_void) == 0;
                        if switched {
                            ABC_ENABLED_BY_US.store(true, Ordering::Relaxed);
                        }
                        CFRelease(all);
                        return switched;
                    }
                }
                CFRelease(all);
            }

            // ③ 兜底：其它 Apple 布局 → Apple 自家输入法（覆盖「系统里没有
            // ABC 布局」的极端情况）。布局与输入法都按身份串前缀识别。
            let list = TISCreateInputSourceList(std::ptr::null(), false);
            if list.is_null() {
                return false;
            }
            let count = CFArrayGetCount(list);
            let mut apple_layout: *const c_void = std::ptr::null();
            let mut apple_input_method: *const c_void = std::ptr::null();
            for index in 0..count {
                let item = CFArrayGetValueAtIndex(list, index);
                if item.is_null() {
                    continue;
                }
                let Some(id) = source_identity(item as *mut c_void) else {
                    continue;
                };
                if apple_layout.is_null() && id.starts_with("com.apple.keylayout.") {
                    apple_layout = item;
                }
                if apple_input_method.is_null() && id.starts_with("com.apple.inputmethod.") {
                    apple_input_method = item;
                }
            }
            let target = if !apple_layout.is_null() {
                apple_layout
            } else {
                apple_input_method
            };
            let selected = !target.is_null() && TISSelectInputSource(target as *mut c_void) == 0;
            CFRelease(list);
            selected
        }
    }

    /// 恢复输入源到备份时的身份串（可能是输入法的 BundleID，也可能是布局的
    /// InputSourceID，与备份时 `current_bundle_id` 的取法一致，因此用
    /// `include_all_installed = true` 拿到完整列表再按身份串匹配）。找不到或
    /// 系统拒绝时不强行干预——输入法留在 ABC 无害，用户手动可切。
    ///
    /// 恢复成功后，若当初的 ABC 是我们临时启用的，顺手把它关回去（用户输入法
    /// 菜单不留我们添加的条目）；ABC 本来就启用的情况不动。
    pub fn switch_to_bundle(bundle_id: &str) -> bool {
        unsafe {
            let list = TISCreateInputSourceList(std::ptr::null(), true);
            if list.is_null() {
                return false;
            }
            let count = CFArrayGetCount(list);
            let mut selected = false;
            for index in 0..count {
                let item = CFArrayGetValueAtIndex(list, index);
                if item.is_null() {
                    continue;
                }
                if source_identity(item as *mut c_void).as_deref() == Some(bundle_id) {
                    selected = TISSelectInputSource(item as *mut c_void) == 0;
                    break;
                }
            }
            CFRelease(list);
            if selected && ABC_ENABLED_BY_US.swap(false, Ordering::Relaxed) {
                disable_abc_layout();
            }
            selected
        }
    }

    /// 把 ABC 布局关回去（仅当它是我们临时启用的情况）。失败静默——多一个
    /// 可用的 ABC 布局对用户无害，不值得为它报错。
    fn disable_abc_layout() {
        unsafe {
            let all = TISCreateInputSourceList(std::ptr::null(), true);
            if all.is_null() {
                return;
            }
            let count = CFArrayGetCount(all);
            for index in 0..count {
                let item = CFArrayGetValueAtIndex(all, index);
                if item.is_null() {
                    continue;
                }
                if source_identity(item as *mut c_void).as_deref() == Some(ABC_SOURCE_ID) {
                    // 当前选中的就是 ABC 时系统会拒绝禁用——那说明恢复没切走，
                    // 保留启用状态反而正确，静默失败即可。
                    let _ = TISDisableInputSource(item as *mut c_void);
                    break;
                }
            }
            CFRelease(all);
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub fn current_bundle_id() -> Option<String> {
        None
    }

    pub fn switch_to_system_ascii() -> bool {
        false
    }

    pub fn switch_to_bundle(_bundle_id: &str) -> bool {
        false
    }
}

pub use imp::{current_bundle_id, switch_to_bundle, switch_to_system_ascii};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_is_attempted_unless_already_on_abc() {
        // 第三方输入法且尚未托管 → 切换。
        assert!(should_attempt_switch(
            Some("com.tencent.inputmethod.wetype"),
            false
        ));
        // 已经托管过 → 不重复切换（多台设备共享同一次）。
        assert!(!should_attempt_switch(
            Some("com.tencent.inputmethod.wetype"),
            true
        ));
        // 已经是 ABC → 不动用户的选择。
        assert!(!should_attempt_switch(Some("com.apple.keylayout.ABC"), false));
        // X10-41 真机定案：系统拼音等输入法模式同样组字吞键 → 也要切到 ABC。
        assert!(should_attempt_switch(
            Some("com.apple.inputmethod.SCIM.ITABC"),
            false
        ));
        // 其它布局（如 Dvorak）也统一切到 ABC，保证镜像输入可用。
        assert!(should_attempt_switch(Some("com.apple.keylayout.Dvorak"), false));
        // 读不到当前输入源 → 宁可不动。
        assert!(!should_attempt_switch(None, false));
    }

    #[test]
    fn restore_only_happens_when_last_session_ended() {
        // 还有会话在跑（哪怕只剩一台）→ 不恢复，用户可能还在打字。
        assert!(!should_attempt_restore(1, true));
        assert!(!should_attempt_restore(2, true));
        // 没有备份（本来就没切过）→ 无事可做。
        assert!(!should_attempt_restore(0, false));
        // 最后一台结束且有备份 → 恢复。
        assert!(should_attempt_restore(0, true));
    }

    /// 走真实 FFI 的只读冒烟测试（X10-40 教训）：上一轮只用 Python ctypes
    /// 验证了语义，Rust 侧 `addr_of!` 传错指针导致「开始镜像即 SIGSEGV」，
    /// 交付前未发现。本测试在 macOS 上直接调用 `current_bundle_id`，一旦
    /// 属性 key 传参再次出错，会在测试里当场崩溃而不是等用户真机踩雷。
    /// 只读不写：不切换、不恢复，对测试环境零副作用。
    #[cfg(target_os = "macos")]
    #[test]
    fn current_bundle_id_smoke_test_via_real_ffi() {
        let bundle = current_bundle_id().expect("TIS should always report the current keyboard input source");
        assert!(!bundle.is_empty());
        // 合法 bundle id 至少包含一个点（com.apple.* / com.tencent.* 等）。
        assert!(bundle.contains('.'), "unexpected bundle id: {bundle}");
    }
}
