//! One-shot preparation with nonblocking delivery to the render thread.
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub(super) struct Background<T> {
    receiver: Receiver<Result<T, String>>,
}
impl<T: Send + 'static> Background<T> {
    pub fn start(job: impl FnOnce() -> T + Send + 'static) -> Self {
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job))
                .map_err(|_| "No se pudo preparar el escenario gráfico de HiveLive".to_string());
            let _ = sender.send(result);
        });
        Self { receiver }
    }
    pub fn poll(&self) -> Option<Result<T, String>> {
        match self.receiver.try_recv() {
            Ok(value) => Some(value),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(
                "La preparación del escenario gráfico se interrumpió".into(),
            )),
        }
    }
    #[cfg(test)]
    pub fn wait(self, timeout: std::time::Duration) -> Result<T, String> {
        self.receiver
            .recv_timeout(timeout)
            .map_err(|_| "La preparación no terminó a tiempo".to_string())?
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn preparation_never_waits_for_a_busy_worker_and_returns_its_result() {
        let (release, wait) = mpsc::channel();
        let main = std::thread::current().id();
        let pending = Background::start(move || {
            wait.recv().unwrap();
            (42, std::thread::current().id())
        });
        assert!(
            pending.poll().is_none(),
            "la UI continúa mientras el trabajo está bloqueado"
        );
        release.send(()).unwrap();
        let (answer, worker) = pending.wait(Duration::from_secs(3)).unwrap();
        assert_eq!(answer, 42);
        assert_ne!(worker, main);
    }
    #[test]
    fn a_failed_preparation_is_reported_instead_of_crashing_the_ui() {
        let pending = Background::<()>::start(|| panic!("fixture graphics failure"));
        assert!(pending.wait(Duration::from_secs(3)).is_err());
    }
}
