//! Latest-frame mailbox: previews stay local and capture never queues old images.
use hive_media::{FrameKind, VideoFrame};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
#[derive(Default)]
struct Inbox {
    epoch: u64,
    active: bool,
    jpeg: Option<Vec<u8>>,
}
#[derive(Clone)]
pub struct Feed {
    inner: Arc<Mutex<Inbox>>,
    epoch: u64,
}
impl Feed {
    pub fn active(&self) -> bool {
        let inbox = self.inner.lock().unwrap();
        inbox.active && inbox.epoch == self.epoch
    }
    pub fn frame(&self, frame: VideoFrame, send: &mut impl FnMut(VideoFrame)) {
        let mut inbox = self.inner.lock().unwrap();
        if !inbox.active || inbox.epoch != self.epoch {
            return;
        }
        if frame.kind == FrameKind::Preview {
            inbox.jpeg = Some(frame.jpeg);
        } else {
            drop(inbox);
            send(frame);
        }
    }
    pub fn end(&self) {
        let mut inbox = self.inner.lock().unwrap();
        if inbox.epoch == self.epoch {
            inbox.active = false;
            inbox.jpeg = None;
            inbox.epoch += 1;
        }
    }
}
pub struct Preview {
    inner: Arc<Mutex<Inbox>>,
    pub pixels: rune::Pixels,
    pub active: bool,
    pub has_image: bool,
    last: Option<Instant>,
    epoch: u64,
    stale: bool,
}
impl Default for Preview {
    fn default() -> Self {
        Self {
            inner: Default::default(),
            pixels: rune::Pixels::new(),
            active: false,
            has_image: false,
            last: None,
            epoch: 0,
            stale: false,
        }
    }
}
impl Preview {
    pub fn begin(&mut self) -> Feed {
        let mut inbox = self.inner.lock().unwrap();
        inbox.epoch += 1;
        inbox.active = true;
        inbox.jpeg = None;
        self.active = true;
        self.has_image = false;
        self.last = None;
        self.epoch = inbox.epoch;
        Feed {
            inner: self.inner.clone(),
            epoch: inbox.epoch,
        }
    }
    pub fn stop(&mut self) {
        let mut inbox = self.inner.lock().unwrap();
        inbox.epoch += 1;
        inbox.active = false;
        inbox.jpeg = None;
        self.active = false;
        self.has_image = false;
        self.last = None;
    }
    pub fn poll(&mut self) -> bool {
        let (active, epoch, jpeg) = {
            let mut inbox = self.inner.lock().unwrap();
            (inbox.active, inbox.epoch, inbox.jpeg.take())
        };
        let mut changed = active != self.active || epoch != self.epoch;
        self.active = active;
        self.epoch = epoch;
        if !active {
            self.has_image = false;
            self.last = None;
            return changed;
        }
        if let Some(jpeg) = jpeg {
            if let Ok(image) = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg)
            {
                let rgba = image.to_rgba8();
                self.pixels
                    .set(rgba.width(), rgba.height(), rgba.into_raw());
                self.has_image = true;
                self.last = Some(Instant::now());
                changed = true;
            }
        }
        let stale = self
            .last
            .is_some_and(|at| at.elapsed() > Duration::from_secs(5));
        changed |= stale != self.stale;
        self.stale = stale;
        changed
    }
    pub fn status(&self) -> &'static str {
        match self.last {
            None => "Esperando imagen…",
            Some(at) if at.elapsed() > Duration::from_secs(5) => "Sin fotogramas recientes",
            _ => "Vista previa local",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn previews_are_latest_only_local_and_late_frames_are_rejected() {
        let mut preview = Preview::default();
        let feed = preview.begin();
        let mut sent = vec![];
        let mut send = |frame: VideoFrame| sent.push(frame.jpeg);
        for byte in [1, 2] {
            feed.frame(
                VideoFrame {
                    kind: FrameKind::Preview,
                    jpeg: vec![byte],
                },
                &mut send,
            );
        }
        assert_eq!(
            preview.inner.lock().unwrap().jpeg.as_deref(),
            Some(&[2][..])
        );
        feed.frame(
            VideoFrame {
                kind: FrameKind::Model,
                jpeg: vec![3],
            },
            &mut send,
        );
        preview.stop();
        let _new = preview.begin();
        feed.frame(
            VideoFrame {
                kind: FrameKind::Model,
                jpeg: vec![4],
            },
            &mut send,
        );
        assert_eq!(sent, vec![vec![3]]);
        assert!(preview.inner.lock().unwrap().jpeg.is_none());
    }
    #[test]
    fn captured_jpeg_is_decoded_for_display_and_cleared_on_end() {
        let mut preview = Preview::default();
        let feed = preview.begin();
        let jpeg = hive_media::video::encode_jpeg(
            &[255; 16],
            2,
            2,
            hive_media::video::Pixels::Rgba,
            hive_media::video::CAMERA_PREVIEW,
        )
        .unwrap();
        feed.frame(
            VideoFrame {
                kind: FrameKind::Preview,
                jpeg,
            },
            &mut |_| panic!("preview left the app"),
        );
        assert!(preview.poll());
        assert!(preview.active && preview.has_image);
        feed.end();
        assert!(preview.poll());
        assert!(!preview.active && !preview.has_image);
    }
}
