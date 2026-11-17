use std::{
    collections::HashMap,
    fmt,
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use pipewire::{
    channel,
    context::ContextRc,
    keys::{MEDIA_CATEGORY, MEDIA_ROLE, MEDIA_TYPE},
    main_loop::MainLoopRc,
    properties,
    spa::{
        param::{
            ParamType,
            format::{FormatProperties, MediaSubtype, MediaType},
            format_utils,
            video::{VideoFormat, VideoInfoRaw},
        },
        pod::{self, Pod, serialize::PodSerializer},
        utils::{Direction, Fraction, Rectangle, SpaTypes},
    },
    stream::{StreamFlags, StreamRc},
};
use zbus::{
    blocking::Proxy,
    zvariant::{DeserializeDict, OwnedFd, OwnedObjectPath, OwnedValue, Type, Value},
};

use crate::{XCapError, XCapResult, video_recorder::Frame};

use super::{
    impl_monitor::ImplMonitor,
    utils::{get_zbus_connection, get_zbus_portal_request},
};

#[allow(dead_code)]
#[derive(DeserializeDict, Type, Debug)]
#[zvariant(signature = "dict")]
pub struct ScreenCastCreateSessionResponse {
    session_handle: String,
}

#[allow(dead_code)]
#[derive(DeserializeDict, Type, Debug)]
#[zvariant(signature = "dict")]
pub struct ScreenCastStartStream {
    pub id: Option<String>,
    pub position: Option<(i32, i32)>,
    pub size: Option<(i32, i32)>,
    pub source_type: Option<u32>,
    pub mapping_id: Option<String>,
}

#[derive(DeserializeDict, Type, Debug)]
#[zvariant(signature = "dict")]
pub struct ScreenCastStartResponse {
    pub streams: Option<Vec<(u32, ScreenCastStartStream)>>,
    #[allow(dead_code)]
    pub restore_token: Option<String>,
}

// Subscribe synchronously before making the request; a fast portal response must
// not race a subscription created on another thread.
fn portal_response<T>(responses: &mut impl Iterator<Item=zbus::Message>)->XCapResult<T>
where T: for<'de> serde::Deserialize<'de> + Type {
    let message=responses.next().ok_or(XCapError::new("Portal closed before responding"))?;
    let (code,_):(u32,HashMap<String,OwnedValue>)=message.body().deserialize()?;
    match code {
        0=>Ok(message.body().deserialize::<(u32,T)>()?.1),
        1=>Err(XCapError::new("Screen sharing canceled")),
        _=>Err(XCapError::new(format!("Screen sharing denied (portal response {code})"))),
    }
}

/// https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html
pub struct ScreenCast<'a> {
    proxy: Proxy<'a>,
}

impl ScreenCast<'_> {
    pub fn new() -> XCapResult<Self> {
        let conn = get_zbus_connection()?;
        let proxy = Proxy::new(
            &conn,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.ScreenCast",
        )?;

        Ok(ScreenCast { proxy })
    }

    pub fn create_session(&self) -> XCapResult<OwnedObjectPath> {
        let conn = self.proxy.connection();

        let mut options = HashMap::new();

        let handle_token = rand::random::<u32>().to_string();
        let portal_request = get_zbus_portal_request(conn, &handle_token)?;

        options.insert("handle_token", Value::from(&handle_token));

        let session_handle_token = rand::random::<u32>().to_string();
        options.insert("session_handle_token", Value::from(&session_handle_token));

        let mut responses = portal_request.receive_signal("Response")?;

        self.proxy.call_method("CreateSession", &(options))?;

        let response: ScreenCastCreateSessionResponse = portal_response(&mut responses)?;

        let unique_name = conn
            .unique_name()
            .ok_or(XCapError::new("Failed to get unique name"))?;
        let unique_identifier = unique_name.trim_start_matches(':').replace('.', "_");

        let session = OwnedObjectPath::try_from(format!(
            "/org/freedesktop/portal/desktop/session/{unique_identifier}/{session_handle_token}"
        ))?;

        if session.as_str() != response.session_handle {
            return Err(XCapError::new("Session handle mismatch"));
        }

        Ok(session)
    }

    pub fn select_sources(&self, session: &OwnedObjectPath) -> XCapResult<()> {
        let conn = self.proxy.connection();

        let mut options = HashMap::new();

        let handle_token = rand::random::<u32>().to_string();
        let portal_request = get_zbus_portal_request(conn, &handle_token)?;

        options.insert("handle_token", Value::from(handle_token));
        options.insert("types", Value::from(1_u32));
        options.insert("multiple", Value::from(false));

        let mut responses = portal_request.receive_signal("Response")?;
        self.proxy.call_method("SelectSources", &(session, options))?;
        let _: HashMap<String, OwnedValue> = portal_response(&mut responses)?;

        Ok(())
    }

    pub fn start(&self, session: &OwnedObjectPath) -> XCapResult<ScreenCastStartResponse> {
        let conn = self.proxy.connection();

        let mut options = HashMap::new();

        let handle_token = rand::random::<u32>().to_string();
        let portal_request = get_zbus_portal_request(conn, &handle_token)?;

        options.insert("handle_token", Value::from(&handle_token));

        let mut responses = portal_request.receive_signal("Response")?;

        self.proxy.call_method("Start", &(session, "", options))?;

        portal_response(&mut responses)
    }

    #[allow(dead_code)]
    pub fn open_pipe_wire_remote(&self, session: &OwnedObjectPath) -> XCapResult<OwnedFd> {
        let options: HashMap<&str, Value<'_>> = HashMap::new();
        let fd: OwnedFd = self.proxy.call("OpenPipeWireRemote", &(session, options))?;

        Ok(fd)
    }
}

// The portal session grants access to the selected stream for its lifetime.
struct PortalSession { portal: ScreenCast<'static>, path: OwnedObjectPath }
impl Drop for PortalSession {
    fn drop(&mut self) {
        if let Ok(proxy) = Proxy::new(self.portal.proxy.connection(), "org.freedesktop.portal.Desktop", self.path.as_str(), "org.freedesktop.portal.Session") {
            let _: Result<(), _> = proxy.call("Close", &());
        }
    }
}

#[derive(Clone)]
pub struct WaylandVideoRecorder {
    #[allow(dead_code)]
    monitor: ImplMonitor,
    sender: Sender<Frame>,
    is_running: Arc<AtomicBool>,
    active_sender: channel::Sender<bool>,
    _portal_session: Arc<PortalSession>,
}

impl fmt::Debug for WaylandVideoRecorder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WaylandVideoRecorder")
            .field("monitor", &self.monitor)
            .field("sender", &self.sender)
            .field("is_running", &self.is_running)
            // Sender is not Debug
            // .field("control_tx", &self.control_tx)
            .finish()
    }
}

#[derive(Clone)]
struct ListenerUserData {
    pub format: VideoInfoRaw,
}

impl WaylandVideoRecorder {
    pub fn new(monitor: ImplMonitor) -> XCapResult<(Self, Receiver<Frame>)> {
        let (sender, receiver) = mpsc::channel();
        let (active_sender, active_receiver) = channel::channel();

        let screen_cast = ScreenCast::new()?;
        let session = screen_cast.create_session()?;
        let portal_session = Arc::new(PortalSession { portal: screen_cast, path: session });
        portal_session.portal.select_sources(&portal_session.path)?;
        let response = portal_session.portal.start(&portal_session.path)?;
        let remote = portal_session.portal.open_pipe_wire_remote(&portal_session.path)?;

        // 获取流节点ID
        let stream_id = response
            .streams
            .ok_or(XCapError::new("Stream ID not found"))?
            .first()
            .ok_or(XCapError::new("Stream ID not found"))?
            .0;

        let recorder = Self {
            monitor,
            sender,
            is_running: Arc::new(AtomicBool::new(false)),
            active_sender,
            _portal_session: portal_session,
        };

        recorder.pipewire_capturer(stream_id, active_receiver, remote.into())?;

        Ok((recorder, receiver))
    }

    pub fn pipewire_capturer(
        &self,
        stream_id: u32,
        active_receiver: channel::Receiver<bool>,
        remote: std::os::fd::OwnedFd,
    ) -> XCapResult<()> {
        let sender = self.sender.clone();
        let is_running = self.is_running.clone();

        thread::spawn(move || {
            pipewire::init();

            let main_loop = MainLoopRc::new(None)?;
            let context = ContextRc::new(&main_loop, None)?;
            let core = context.connect_fd_rc(remote, None)?;

            let user_data = ListenerUserData {
                format: Default::default(),
            };

            let stream = StreamRc::new(
                core.clone(),
                "XCap",
                properties::properties! {
                    *MEDIA_TYPE => "Video",
                    *MEDIA_CATEGORY => "Capture",
                    *MEDIA_ROLE => "Screen",
                },
            )?;

            let close_loop=main_loop.clone();
            let _listener = stream
                .add_local_listener_with_user_data(user_data)
                .state_changed(move |_,_,_,state| {
                    if matches!(state,pipewire::stream::StreamState::Error(_) | pipewire::stream::StreamState::Unconnected) {close_loop.quit();}
                })
                .param_changed(|_, user_data, id, param| {
                    let Some(param) = param else {
                        return;
                    };

                    if id != ParamType::Format.as_raw() {
                        return;
                    }

                    let (media_type, media_subtype) = match format_utils::parse_format(param) {
                        Ok(v) => v,
                        Err(err) => {
                            log::error!("Failed to parse format: {err:?}");
                            return;
                        }
                    };

                    if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
                        return;
                    }

                    if let Err(err) = user_data.format.parse(param) {
                        log::error!("Failed to parse format: {err:?}");
                    }
                })
                .process(move |stream, user_data| {
                    let state = is_running.load(Ordering::Relaxed);
                    match stream.dequeue_buffer() {
                        None => log::info!("stream.dequeue_buffer() returned None"),
                        Some(mut buffer) => {
                            let datas = buffer.datas_mut();
                            if datas.is_empty() {
                                return;
                            }
                            let size = user_data.format.size();
                            if let Some(frame_data) = datas[0].data() {
                                let buffer = match user_data.format.format() {
                                    VideoFormat::RGB => {
                                        let mut buf =
                                            vec![0; (size.width * size.height * 4) as usize];
                                        for (src, dst) in frame_data
                                            .as_chunks::<3>()
                                            .0
                                            .iter()
                                            .zip(buf.as_chunks_mut::<4>().0)
                                        {
                                            dst[0] = src[0];
                                            dst[1] = src[1];
                                            dst[2] = src[2];
                                            dst[3] = 255;
                                        }

                                        buf
                                    }
                                    VideoFormat::RGBA => frame_data.to_vec(),
                                    VideoFormat::RGBx => frame_data.to_vec(),
                                    VideoFormat::BGRx => {
                                        let mut buf = frame_data.to_vec();
                                        for src in buf.as_chunks_mut::<4>().0 {
                                            src.swap(0, 2);
                                        }

                                        buf
                                    }
                                    _ => {
                                        log::error!(
                                            "Unsupported format: {:?}",
                                            user_data.format.format()
                                        );
                                        return;
                                    }
                                };

                                if state {
                                    let _ =
                                        sender.send(Frame::new(size.width, size.height, buffer));
                                }
                            }
                        }
                    }
                })
                .register()?;

            let obj = pod::object!(
                SpaTypes::ObjectParamFormat,
                ParamType::EnumFormat,
                pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
                pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
                pod::property!(
                    FormatProperties::VideoFormat,
                    Choice,
                    Enum,
                    Id,
                    VideoFormat::RGB,
                    VideoFormat::RGBA,
                    VideoFormat::RGBx,
                    VideoFormat::BGRx,
                    // VideoFormat::YUY2,
                    // VideoFormat::I420,
                ),
                pod::property!(
                    FormatProperties::VideoSize,
                    Choice,
                    Range,
                    Rectangle,
                    Rectangle {
                        width: 128,
                        height: 128
                    },
                    Rectangle {
                        width: 1,
                        height: 1
                    },
                    Rectangle {
                        width: 4096,
                        height: 4096
                    }
                ),
                pod::property!(
                    FormatProperties::VideoFramerate,
                    Choice,
                    Range,
                    Fraction,
                    Fraction { num: 24, denom: 1 },
                    Fraction { num: 0, denom: 1 },
                    Fraction {
                        num: 1000,
                        denom: 1
                    }
                ),
            );
            let values =
                PodSerializer::serialize(Cursor::new(Vec::new()), &pod::Value::Object(obj))
                    .map_err(XCapError::new)?
                    .0
                    .into_inner();

            let mut params =
                [Pod::from_bytes(&values).ok_or(XCapError::new("Failed to create Pod"))?];

            stream.connect(
                Direction::Input,
                Some(stream_id),
                StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
                &mut params,
            )?;

            // Used to pause/resume the stream
            let _attached = active_receiver.attach(main_loop.loop_(), {
                move |active| {
                    if let Err(e) = stream.set_active(active) {
                        log::error!("Failed to set stream active={active}: {e:?}");
                    }
                    if !active && let Err(e) = stream.flush(true) {
                        log::error!("Failed to flush: {e:?}");
                    }
                }
            });

            main_loop.run();

            Result::<(), XCapError>::Ok(())
        });

        Ok(())
    }

    pub fn start(&self) -> XCapResult<()> {
        self.is_running.store(true, Ordering::Relaxed);
        let _ = self.active_sender.send(true);
        Ok(())
    }

    pub fn stop(&self) -> XCapResult<()> {
        self.is_running.store(false, Ordering::Relaxed);
        let _ = self.active_sender.send(false);
        Ok(())
    }
}

#[cfg(test)]
mod hive_tests {
    use super::*;
    fn response(code:u32)->zbus::Message {
        zbus::Message::signal("/org/freedesktop/portal/desktop/request/test","org.freedesktop.portal.Request","Response").unwrap()
            .build(&(code,HashMap::<String,OwnedValue>::new())).unwrap()
    }
    #[test]
    fn hive_portal_decodes_success_and_distinguishes_cancel_and_denial() {
        let result:HashMap<String,OwnedValue>=portal_response(&mut vec![response(0)].into_iter()).unwrap();
        assert!(result.is_empty());
        for (code,expected) in [(1,"canceled"),(2,"denied")] {
            let error=portal_response::<HashMap<String,OwnedValue>>(&mut vec![response(code)].into_iter()).unwrap_err();
            assert!(error.to_string().contains(expected));
        }
        assert!(portal_response::<HashMap<String,OwnedValue>>(&mut std::iter::empty()).is_err());
    }
}
