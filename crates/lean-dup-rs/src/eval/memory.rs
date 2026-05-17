#[cfg(target_os = "macos")]
pub(crate) fn peak_rss_bytes() -> Option<u64> {
    getrusage_peak_rss()
}

#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn peak_rss_bytes() -> Option<u64> {
    getrusage_peak_rss().map(|kilobytes| kilobytes.saturating_mul(1024))
}

#[cfg(not(unix))]
pub(crate) fn peak_rss_bytes() -> Option<u64> {
    None
}

#[cfg(unix)]
fn getrusage_peak_rss() -> Option<u64> {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct TimeVal {
        tv_sec: isize,
        tv_usec: isize,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RUsage {
        ru_utime: TimeVal,
        ru_stime: TimeVal,
        ru_maxrss: isize,
        ru_ixrss: isize,
        ru_idrss: isize,
        ru_isrss: isize,
        ru_minflt: isize,
        ru_majflt: isize,
        ru_nswap: isize,
        ru_inblock: isize,
        ru_oublock: isize,
        ru_msgsnd: isize,
        ru_msgrcv: isize,
        ru_nsignals: isize,
        ru_nvcsw: isize,
        ru_nivcsw: isize,
    }

    unsafe extern "C" {
        fn getrusage(who: i32, usage: *mut RUsage) -> i32;
    }

    const RUSAGE_SELF: i32 = 0;
    let mut usage = RUsage {
        ru_utime: TimeVal { tv_sec: 0, tv_usec: 0 },
        ru_stime: TimeVal { tv_sec: 0, tv_usec: 0 },
        ru_maxrss: 0,
        ru_ixrss: 0,
        ru_idrss: 0,
        ru_isrss: 0,
        ru_minflt: 0,
        ru_majflt: 0,
        ru_nswap: 0,
        ru_inblock: 0,
        ru_oublock: 0,
        ru_msgsnd: 0,
        ru_msgrcv: 0,
        ru_nsignals: 0,
        ru_nvcsw: 0,
        ru_nivcsw: 0,
    };

    let status = unsafe { getrusage(RUSAGE_SELF, &mut usage) };
    (status == 0 && usage.ru_maxrss >= 0).then_some(usage.ru_maxrss as u64)
}
