# Local patch to xcap 0.9.8 (Apache-2.0)

Upstream source is unchanged except `src/linux/wayland_video_recorder.rs`:
- Subscribe before portal requests and await their response, including SelectSources,
  avoiding fast-response races and reporting cancellation explicitly.
- OpenPipeWireRemote and connect via the authorized file descriptor.
- Retain and explicitly close the portal session with the recorder lifetime.
- End the PipeWire loop on stream disconnection/error so the app observes the end.

The public API and other operating system backends are unchanged. Recheck these
changes when upgrading xcap; remove this patch when upstream provides equivalent fixes.
Reference: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html
