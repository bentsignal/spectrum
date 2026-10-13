//! Opening the player: the picture comes from footage's playback copy when
//! it has one (fast to scrub, play backward, and play on any computer), and
//! a paused frame is then redrawn from the original at full quality.
use crate::{audio_out::AudioOut, player::Player, player_decode, workspace::Workspace};
use gpui::*;
use spectrum_library::AssetId;
use spectrum_media::Time;
use std::time::Duration;

impl Workspace {
    /// Opens footage or audio in the player, at `at`.
    pub fn load_player(&mut self, id: AssetId, window: &mut Window, cx: &mut Context<Self>) {
        self.open_player_at(id, Time::ZERO, window, cx);
    }

    fn open_player_at(
        &mut self,
        id: AssetId,
        at: Time,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.player = None;
        let Ok(store) = &self.store else {
            return;
        };
        let found = store.service.media(id).and_then(|(media, file)| {
            let copy = store.service.playback_copy(id)?;
            Ok((media, file, copy))
        });
        let (media, file, copy) = match found {
            Ok(found) => found,
            Err(error) => return self.notify_error(error, window, cx),
        };
        let size = self.player_size(window);
        let mut decoders = None;
        if let Some(video) = &media.info.video {
            // The copy's frames are turned as the original's are.
            let source = copy.clone().unwrap_or_else(|| file.clone());
            let rotation = copy.as_ref().map(|_| video.rotation);
            let main = player_decode::start(source, rotation);
            let sharp = copy
                .as_ref()
                .map(|_| player_decode::start(file.clone(), None));
            match (main, sharp.transpose()) {
                (Ok(main), Ok(sharp)) => decoders = Some((main, sharp)),
                (Err(error), _) | (_, Err(error)) => return self.notify_error(error, window, cx),
            }
        }
        let audio = (!media.info.audio.is_empty()).then(|| AudioOut::open(file.clone()));
        let has_picture = media.info.video.is_some();
        let duration = media.info.duration;
        let (main, sharp) = match decoders {
            Some(((requests, frames), sharp)) => (Some((requests, frames)), sharp),
            None => (None, None),
        };
        let timing = crate::player::Timing::of(&media);
        let mut player = Player::new(
            id,
            Some(media),
            timing,
            size,
            main.as_ref().map(|(requests, _)| requests.clone()),
            sharp.as_ref().map(|(requests, _)| requests.clone()),
            audio,
        );
        player.seek(at);
        self.player = Some(player);
        if self.player_controls.is_none() {
            self.player_controls = Some(crate::player_view::PlayerControls::new(window, cx));
        }
        if let Some(controls) = &mut self.player_controls {
            if let Some(old) = controls.waveform.take() {
                crate::retire::drop_later(old);
            }
            controls.shown = None;
        }
        if !has_picture {
            self.draw_waveform(id, file, duration, window, cx);
        }
        if let Some((_, frames)) = main {
            self.receive_frames(id, frames, false, window, cx);
        }
        if let Some((_, frames)) = sharp {
            self.receive_frames(id, frames, true, window, cx);
        }
    }

    /// Hands decoded frames to the player as they arrive, keeping playback's
    /// frames in the decoder until there is room, so few wait in memory.
    fn receive_frames(
        &mut self,
        id: AssetId,
        frames: flume::Receiver<player_decode::Delivery>,
        sharp: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                while this
                    .read_with(cx, |ws, _| ws.player.as_ref().is_some_and(Player::full))
                    .unwrap_or(false)
                {
                    cx.background_executor()
                        .timer(Duration::from_millis(4))
                        .await;
                }
                let Ok(delivery) = frames.recv_async().await else {
                    break;
                };
                let delivered = this.update_in(cx, |this, window, cx| {
                    let frame = match delivery {
                        Ok(frame) => frame,
                        Err(error) => return this.notify_error(error, window, cx),
                    };
                    match this.player.as_mut().filter(|p| p.id == id) {
                        Some(player) if sharp => player.receive_sharp(frame),
                        Some(player) => player.receive(frame),
                        None => crate::retire::drop_later(frame.image),
                    }
                    cx.notify();
                });
                if delivered.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn draw_waveform(
        &mut self,
        id: AssetId,
        file: std::path::PathBuf,
        duration: Time,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let drawn = cx
            .background_executor()
            .spawn(async move { crate::player_view::draw_waveform(&file, 0, duration) });
        cx.spawn_in(window, async move |this, cx| {
            let drawn = drawn.await;
            this.update_in(cx, |this, window, cx| match drawn {
                Ok(image) if this.player.as_ref().is_some_and(|p| p.id == id) => {
                    if let Some(controls) = &mut this.player_controls
                        && let Some(old) = controls.waveform.replace(image)
                    {
                        crate::retire::drop_later(old);
                    }
                    cx.notify();
                }
                Ok(image) => crate::retire::drop_later(image),
                Err(error) => this.notify_error(error, window, cx),
            })
            .ok();
        })
        .detach();
    }

    /// A playback copy just finished: the open player switches to it where
    /// it is, paused.
    pub fn copy_ready(&mut self, id: AssetId, cx: &mut Context<Self>) {
        if let Some(player) = &mut self.player
            && player.id == id
        {
            player.copy_arrived = true;
            cx.notify();
        }
    }

    /// Reopens the player on its new copy once one has arrived.
    pub fn switch_to_copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(player) = &self.player else {
            return;
        };
        if !player.copy_arrived || player.playing() {
            return;
        }
        let (id, at) = (player.id, player.now());
        self.open_player_at(id, at, window, cx);
    }
}
