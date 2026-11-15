//! Thumbnails of visual media for the timeline, made in the background and
//! kept as textures.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use eframe::egui;
use vizx_core::DecodedImage;
use vizx_project::MediaSource;

enum Thumbnail {
    Loading,
    /// The texture and its width divided by its height.
    Ready(egui::TextureHandle, f32),
    /// Nothing to show (or it could not be made).
    None,
}

/// Remembers one thumbnail per picture a project shows.
pub struct Thumbnails {
    states: HashMap<String, Thumbnail>,
    sender: Sender<(String, Result<DecodedImage, String>)>,
    receiver: Receiver<(String, Result<DecodedImage, String>)>,
    ffmpeg: Option<PathBuf>,
}

impl Thumbnails {
    pub fn new(ffmpeg: Option<PathBuf>) -> Self {
        let (sender, receiver) = channel();
        Self {
            states: HashMap::new(),
            sender,
            receiver,
            ffmpeg,
        }
    }

    /// The thumbnail of `source` and its shape (width / height), if it has
    /// one and it is ready. The first call starts making it.
    pub fn get(
        &mut self,
        ctx: &egui::Context,
        source: &MediaSource,
    ) -> Option<(egui::TextureId, f32)> {
        if !source.is_visual() || source.is_visualizer() {
            return None;
        }
        self.collect(ctx);
        // What the picture depends on: the file, or for a text all of it.
        let key = match source.text() {
            Some(text) => format!("text:{text:?}"),
            None => format!("{}|{:?}", source.path, source.animation()),
        };
        match self.states.get(&key) {
            Some(Thumbnail::Ready(texture, aspect)) => Some((texture.id(), *aspect)),
            Some(_) => None,
            None => {
                self.states.insert(key.clone(), Thumbnail::Loading);
                let (sender, source, ffmpeg, ctx) = (
                    self.sender.clone(),
                    source.clone(),
                    self.ffmpeg.clone(),
                    ctx.clone(),
                );
                thread::spawn(move || {
                    let picture = vizx_core::make_thumbnail(&source, ffmpeg.as_deref());
                    let _ = sender.send((key, picture));
                    ctx.request_repaint();
                });
                None
            }
        }
    }

    /// Turns finished thumbnails into textures.
    fn collect(&mut self, ctx: &egui::Context) {
        while let Ok((key, picture)) = self.receiver.try_recv() {
            let state = match picture {
                Ok(picture) if picture.width > 0 && picture.height > 0 => {
                    let image = egui::ColorImage::from_rgba_premultiplied(
                        [picture.width as usize, picture.height as usize],
                        &picture.pixels,
                    );
                    let texture = ctx.load_texture(
                        format!("thumbnail {key}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    Thumbnail::Ready(texture, picture.width as f32 / picture.height as f32)
                }
                _ => Thumbnail::None,
            };
            self.states.insert(key, state);
        }
    }
}
