//! Hold a macOS power assertion (`PreventUserIdleSystemSleep`) while mining, so a
//! Mac configured to sleep when idle does not system-sleep mid-mine and stop
//! earning. This is the companion to `LSAppNapIsDisabled` (Info.plist): App Nap
//! suppression keeps the mining loop from being THROTTLED when the app is
//! backgrounded/locked; this assertion keeps the machine from SLEEPING when the
//! user is away. Display sleep is intentionally NOT prevented — the GPU keeps
//! mining with the screen off, so we only block *system* idle sleep.
//!
//! RAII: hold the guard for the duration of a mining session; dropping it
//! releases the assertion so the Mac can sleep normally once mining stops. Off
//! macOS the guard is an inert zero-sized value, so callers stay platform-free.

#[cfg(target_os = "macos")]
mod imp {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;
    use core_foundation_sys::string::CFStringRef;

    // IOKit owns the power-assertion API. Linking the framework is enough — no
    // entitlement or sudo is needed to assert "don't idle-sleep" on AC/battery.
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPMAssertionCreateWithName(
            assertion_type: CFStringRef,
            assertion_level: u32,
            assertion_name: CFStringRef,
            assertion_id: *mut u32,
        ) -> i32; // IOReturn
        fn IOPMAssertionRelease(assertion_id: u32) -> i32; // IOReturn
    }

    const K_IOPM_ASSERTION_LEVEL_ON: u32 = 255; // kIOPMAssertionLevelOn
    const K_IORETURN_SUCCESS: i32 = 0;

    /// Create a `PreventUserIdleSystemSleep` assertion. Returns its id, or 0 if
    /// the OS refused — 0 is the inert "nothing held" sentinel (release no-ops).
    pub(super) fn create(name: &str) -> u32 {
        let atype = CFString::new("PreventUserIdleSystemSleep");
        let aname = CFString::new(name);
        let mut id: u32 = 0;
        // SAFETY: standard IOKit power-assertion ABI. `atype`/`aname` outlive the
        // call; `id` is a valid out-pointer. On failure `id` is left 0.
        let rc = unsafe {
            IOPMAssertionCreateWithName(
                atype.as_concrete_TypeRef(),
                K_IOPM_ASSERTION_LEVEL_ON,
                aname.as_concrete_TypeRef(),
                &mut id,
            )
        };
        if rc == K_IORETURN_SUCCESS {
            id
        } else {
            0
        }
    }

    pub(super) fn release(id: u32) {
        if id != 0 {
            // SAFETY: `id` came from a successful `create`; releasing it once is
            // the documented lifecycle.
            unsafe {
                IOPMAssertionRelease(id);
            }
        }
    }
}

/// While this guard is alive, macOS will not idle-sleep the system. Best-effort:
/// if the OS call fails the guard is still returned (inert), so a power-assertion
/// problem can never block or crash mining.
#[must_use = "the assertion is released when this guard is dropped — bind it for the mining session"]
pub struct SleepAssertion {
    #[cfg(target_os = "macos")]
    id: u32,
}

impl SleepAssertion {
    /// Hold a "don't idle-sleep" assertion for `reason` (visible in
    /// `pmset -g assertions`). No-op off macOS.
    pub fn hold(reason: &str) -> Self {
        #[cfg(target_os = "macos")]
        {
            SleepAssertion {
                id: imp::create(reason),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = reason;
            SleepAssertion {}
        }
    }
}

impl Drop for SleepAssertion {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        imp::release(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::SleepAssertion;

    // Holding then dropping the assertion must never panic, on any platform
    // (macOS actually asserts+releases; elsewhere it's a no-op).
    #[test]
    fn hold_and_drop_is_safe() {
        let g = SleepAssertion::hold("easyBTX test assertion");
        drop(g);
        // A second, overlapping hold is also fine (assertions are independent).
        let _a = SleepAssertion::hold("a");
        let _b = SleepAssertion::hold("b");
    }
}
