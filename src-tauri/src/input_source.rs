//! 宿主（macOS）输入源自动切换（X10-39）。
//!
//! 背景：UHID 物理键盘语义下，按键以原始键码直送手机、不经过 Mac 输入法。
//! 第三方输入法（如微信输入法）激活时会把按键事件消费进自己的组字缓冲，
//! scrcpy 窗口收不到任何原始键，用户感知为「打字不生效」（X10-38 真机定案）。
//!
//! 方案：镜像会话开始时，若当前输入源是第三方输入法，临时切换到系统自带的
//! 输入源（ABC 布局 → 其它 Apple 布局 → Apple 自家输入法，逐级兜底）；最后一个
//! 会话结束（或应用退出）时恢复用户原来的输入源。切换动作
//! 通过 Carbon 的 TIS（Text Input Source Services）API 完成，无弹窗、无辅助
//! 功能权限要求。
//!
//! 平台边界：只在 macOS 实现；Windows/Linux 编译为空操作（调用方会得到
//! `None`/`false`，自然跳过一切逻辑）。

/// 判断 bundle id 是否属于第三方输入源（非 Apple 出品）。
///
/// 第三方输入法的 bundle id 一定不以 `com.apple.` 开头；系统自带布局是
/// `com.apple.keylayout.*`，系统自带输入法是 `com.apple.inputmethod.*`。
pub fn is_third_party(bundle_id: &str) -> bool {
    !bundle_id.starts_with("com.apple.")
}

/// 会话开始时的纯决策：是否需要尝试切换输入源。
///
/// `already_managed` 表示本次应用运行期间已经切过一次（有备份）——输入源是
/// 宿主全局的，多台并发设备共享同一次切换，不必重复。
/// 返回 `true` 只代表「值得尝试」；实际切换可能失败（系统拒绝），由调用方兜底。
pub(crate) fn should_attempt_switch(current: Option<&str>, already_managed: bool) -> bool {    if already_managed {
        return false;
    }
    current.is_some_and(is_third_party)
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
    use std::ffi::{c_char, c_int, c_long, c_void, CStr};

    // Carbon/HIToolbox 的 TIS API。TISInputSourceRef 本质是 CFTypeRef（*mut c_void）。
    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        fn TISCopyCurrentKeyboardInputSource() -> *mut c_void;
        fn TISGetInputSourceProperty(source: *mut c_void, key: *const c_void) -> *mut c_void;
        fn TISSelectInputSource(source: *mut c_void) -> c_int; // OSStatus，0 = 成功
        fn TISCreateInputSourceList(
            properties: *const c_void,
            include_all_installed: bool,
        ) -> *mut c_void; // CFArrayRef（调用方持有，需释放）
        static kTISPropertyBundleID: *const c_void; // CFStringRef 属性键，取地址使用
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

    /// 系统自带的美式英文布局：恢复镜像输入能力的首选目标。
    const PREFERRED_ASCII_LAYOUT: &str = "com.apple.keylayout.ABC";

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

    /// 当前键盘输入源的 bundle id。第三方输入法（输入模式）激活时，这里返回
    /// 该输入法自己的 bundle id——正是我们要识别并临时切换掉的对象。
    pub fn current_bundle_id() -> Option<String> {
        unsafe {
            let source = TISCopyCurrentKeyboardInputSource();
            if source.is_null() {
                return None;
            }
            let value = TISGetInputSourceProperty(
                source,
                std::ptr::addr_of!(kTISPropertyBundleID) as *const c_void,
            );
            let result = cfstring_to_string(value);
            CFRelease(source as *const c_void);
            result
        }
    }

    /// 切换到系统自带的输入源（真机实测有效，X10-39）。按三级偏好挑选：
    /// ① ABC 布局；② 任意 Apple 英文布局（`com.apple.keylayout.*`）；
    /// ③ Apple 自家输入法（`com.apple.inputmethod.*`，如简体拼音）——部分
    /// 用户没有启用任何英文布局，只有系统输入法，实测系统输入法不会像第三方
    /// 输入法那样吞掉原始键码。三个层级都找不到时不动用户的选择。
    pub fn switch_to_system_ascii() -> bool {
        unsafe {
            let list = TISCreateInputSourceList(std::ptr::null(), false);
            if list.is_null() {
                return false;
            }
            let count = CFArrayGetCount(list);
            let mut abc: *const c_void = std::ptr::null();
            let mut apple_layout: *const c_void = std::ptr::null();
            let mut apple_input_method: *const c_void = std::ptr::null();
            for index in 0..count {
                let item = CFArrayGetValueAtIndex(list, index);
                if item.is_null() {
                    continue;
                }
                let value = TISGetInputSourceProperty(
                    item as *mut c_void,
                    std::ptr::addr_of!(kTISPropertyBundleID) as *const c_void,
                );
                let Some(id) = cfstring_to_string(value) else {
                    continue;
                };
                if abc.is_null() && id == PREFERRED_ASCII_LAYOUT {
                    abc = item;
                    break;
                }
                if apple_layout.is_null() && id.starts_with("com.apple.keylayout.") {
                    apple_layout = item;
                }
                if apple_input_method.is_null() && id.starts_with("com.apple.inputmethod.") {
                    apple_input_method = item;
                }
            }
            let target = if !abc.is_null() {
                abc
            } else if !apple_layout.is_null() {
                apple_layout
            } else {
                apple_input_method
            };
            let selected = !target.is_null() && TISSelectInputSource(target as *mut c_void) == 0;
            CFRelease(list);
            selected
        }
    }

    /// 按 bundle id 恢复输入源（包括第三方输入法这类「输入模式」，因此要用
    /// `include_all_installed = true` 拿到完整列表）。找不到或系统拒绝时不
    /// 强行干预——输入法留在英文布局无害，用户手动可切。
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
                let value = TISGetInputSourceProperty(
                    item as *mut c_void,
                    std::ptr::addr_of!(kTISPropertyBundleID) as *const c_void,
                );
                let Some(id) = cfstring_to_string(value) else {
                    continue;
                };
                if id == bundle_id {
                    selected = TISSelectInputSource(item as *mut c_void) == 0;
                    break;
                }
            }
            CFRelease(list);
            selected
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
    fn apple_sources_are_not_third_party() {
        assert!(!is_third_party("com.apple.keylayout.ABC"));
        assert!(!is_third_party("com.apple.inputmethod.SCIM.ITABC"));
        assert!(!is_third_party("com.apple.keylayout.US"));
    }

    #[test]
    fn third_party_implementations_are_detected() {
        assert!(is_third_party("com.tencent.inputmethod.wetype"));
        assert!(is_third_party("com.sogou.inputmethod.sogou"));
        assert!(is_third_party("org.unknown.ime"));
    }

    #[test]
    fn switch_is_only_attempted_for_unmanaged_third_party_sources() {
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
        // 系统自带输入法/布局 → 不动用户的选择。
        assert!(!should_attempt_switch(Some("com.apple.keylayout.ABC"), false));
        assert!(!should_attempt_switch(
            Some("com.apple.inputmethod.SCIM.ITABC"),
            false
        ));
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
}
