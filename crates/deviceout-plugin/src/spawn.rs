use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub(crate) fn detach(name: &str, job: impl FnOnce() + Send + 'static) -> bool {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(job)
        .is_ok()
}

pub(crate) fn detach_once(name: &str, busy: &Arc<AtomicBool>, job: impl FnOnce() + Send + 'static) {
    if busy.swap(true, Ordering::AcqRel) {
        return;
    }
    let flag = Arc::clone(busy);
    let started = detach(name, move || {
        job();
        flag.store(false, Ordering::Release);
    });
    if !started {
        busy.store(false, Ordering::Release);
    }
}

pub(crate) fn open_url(target: impl Into<String>) {
    let target = target.into();
    detach("deviceout-open", move || {
        let _apartment = Apartment::enter();
        let _ = open::that_detached(&target);
    });
}

#[cfg(windows)]
struct Apartment(bool);

#[cfg(windows)]
impl Apartment {
    fn enter() -> Self {
        use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
        let hr = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        Self(hr >= 0)
    }
}

#[cfg(windows)]
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { windows_sys::Win32::System::Com::CoUninitialize() };
        }
    }
}

#[cfg(not(windows))]
struct Apartment;

#[cfg(not(windows))]
impl Apartment {
    fn enter() -> Self {
        Self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    const PATIENCE: Duration = Duration::from_secs(5);

    #[test]
    fn the_caller_hands_the_job_over_instead_of_running_it() {
        let (release, hold) = mpsc::channel::<()>();
        let (finished, watch) = mpsc::channel::<()>();

        detach("test-detach", move || {
            hold.recv().expect("放行信号丢了");
            finished.send(()).expect("完成信号丢了");
        });

        release.send(()).expect("任务没有被别的线程接走");
        watch.recv_timeout(PATIENCE).expect("任务没有跑完");
    }

    #[test]
    fn a_second_request_is_dropped_while_the_first_is_still_running() {
        let busy = Arc::new(AtomicBool::new(false));
        let (release, hold) = mpsc::channel::<()>();
        let (ran, watch) = mpsc::channel::<()>();
        let again = ran.clone();

        detach_once("test-once", &busy, move || {
            hold.recv().expect("放行信号丢了");
            ran.send(()).expect("计数丢了");
        });
        detach_once("test-once", &busy, move || {
            again.send(()).expect("计数丢了");
        });

        release.send(()).expect("首个任务没有被接走");
        watch.recv_timeout(PATIENCE).expect("首个任务没有跑完");
        assert!(
            watch.recv_timeout(Duration::from_millis(200)).is_err(),
            "第二次请求也跑了，重复的扫描没有被挡住"
        );
    }

    #[test]
    fn the_guard_clears_once_the_job_is_done() {
        let busy = Arc::new(AtomicBool::new(false));
        let (ran, watch) = mpsc::channel::<()>();

        detach_once("test-once", &busy, move || {
            ran.send(()).expect("完成信号丢了");
        });
        watch.recv_timeout(PATIENCE).expect("任务没有跑完");

        let deadline = Instant::now() + PATIENCE;
        while busy.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(
            !busy.load(Ordering::Acquire),
            "任务结束了标记却没清掉，之后再也发不出请求"
        );
    }
}
