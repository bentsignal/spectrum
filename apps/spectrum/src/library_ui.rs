use crate::{SpectrumApp, WorkspaceKind};
use eframe::egui;
use spectrum::library::Service;
use spectrum_library::{Asset, AssetId};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};

type CanvasSummary = (PathBuf, String, Vec<(String, AssetId)>);
enum Request {
    Poll(Vec<AssetId>, Vec<CanvasSummary>),
    Place(AssetId, PathBuf),
    Adopt(u64, PathBuf, PathBuf),
    Copy(AssetId),
}
enum Action {
    None,
    Place(Asset, PathBuf, PathBuf),
    Adopt(Asset, u64, PathBuf),
    Open(Asset),
}
struct Update {
    assets: Vec<Asset>,
    previews: HashMap<AssetId, PathBuf>,
    action: Action,
}
pub(crate) struct LibraryUi {
    root: PathBuf,
    sender: Sender<Request>,
    receiver: Receiver<Result<Update, String>>,
    assets: Vec<Asset>,
    previews: HashMap<AssetId, PathBuf>,
    busy: bool,
    poll_at: Instant,
    status: String,
}
impl LibraryUi {
    pub(crate) fn new(root: PathBuf, context: egui::Context) -> Self {
        let (sender, requests) = mpsc::channel();
        let (responses, receiver) = mpsc::channel();
        let worker_root = root.clone();
        std::thread::spawn(move || {
            let mut service = match Service::open(&worker_root) {
                Ok(s) => s,
                Err(e) => {
                    let _ = responses.send(Err(format!("{e:#}")));
                    context.request_repaint();
                    return;
                }
            };
            while let Ok(request) = requests.recv() {
                let result = (|| -> anyhow::Result<Update> {
                    if let Request::Poll(_, summaries) = &request {
                        for (path, name, links) in summaries {
                            service.index_canvas(path, name, links)?;
                        }
                    }
                    let assets = service.scan()?;
                    let mut previews = HashMap::new();
                    let action = match request {
                        Request::Poll(ids, _) => {
                            for id in ids {
                                previews.insert(id, service.preview(id)?);
                            }
                            Action::None
                        }
                        Request::Place(id, canvas) => {
                            Action::Place(service.library.get(id)?, service.preview(id)?, canvas)
                        }
                        Request::Adopt(layer, path, canvas) => {
                            let asset = service
                                .import(vec![path])?
                                .pop()
                                .ok_or_else(|| anyhow::anyhow!("image import produced no asset"))?;
                            Action::Adopt(asset, layer, canvas)
                        }
                        Request::Copy(id) => Action::Open(service.copy(id)?),
                    };
                    Ok(Update {
                        assets,
                        previews,
                        action,
                    })
                })()
                .map_err(|e| format!("{e:#}"));
                if responses.send(result).is_err() {
                    break;
                }
                context.request_repaint();
            }
        });
        Self {
            root,
            sender,
            receiver,
            assets: Vec::new(),
            previews: HashMap::new(),
            busy: false,
            poll_at: Instant::now(),
            status: String::new(),
        }
    }
    fn send(&mut self, request: Request) {
        if !self.busy {
            self.busy = self.sender.send(request).is_ok();
            if self.busy {
                self.status = "Updating library…".into();
            }
        }
    }
}
impl SpectrumApp {
    fn open_asset(&mut self, asset: &Asset, context: &egui::Context) {
        let path = self.library.root.join(&asset.document);
        if asset.kind == "image" {
            if let Some(id) = asset.item {
                self.photo.library_open(path, id);
                self.switch_to(WorkspaceKind::Photo, context);
            }
        } else if asset.kind == "canvas" {
            self.canvas.library_open(&path);
            self.switch_to(WorkspaceKind::Canvas, context);
        }
    }
    pub(crate) fn library_poll(&mut self, context: &egui::Context) {
        while let Ok(result) = self.library.receiver.try_recv() {
            self.library.busy = false;
            match result {
                Err(error) => self.library.status = error,
                Ok(update) => {
                    self.library.assets = update.assets;
                    self.library.previews.extend(update.previews);
                    self.library.status.clear();
                    match update.action {
                        Action::None => {}
                        Action::Place(asset, path, canvas) => {
                            if self.canvas.library_path().as_ref() == Some(&canvas) {
                                self.canvas.library_place(asset.id, path, asset.name);
                                self.switch_to(WorkspaceKind::Canvas, context);
                            } else {
                                self.library.status =
                                    "Canvas changed; place the image again.".into();
                            }
                        }
                        Action::Adopt(asset, layer, canvas) => {
                            if self.canvas.library_path().as_ref() == Some(&canvas) {
                                self.canvas.library_link(layer, asset.id);
                                self.open_asset(&asset, context);
                            } else {
                                self.library.status =
                                    "Image imported; select it in Library.".into();
                            }
                        }
                        Action::Open(asset) => self.open_asset(&asset, context),
                    }
                }
            }
        }
        self.canvas.library_refresh(&self.library.previews);
        if Instant::now() >= self.library.poll_at && !self.library.busy {
            self.library.poll_at = Instant::now() + Duration::from_millis(750);
            self.library.send(Request::Poll(
                self.canvas.library_references(),
                self.canvas.library_summaries(),
            ));
        }
        context.request_repaint_after(Duration::from_millis(100));
    }
    pub(crate) fn library_controls(&mut self, ui: &mut egui::Ui) {
        let selected = match self.active {
            WorkspaceKind::Photo => self.photo.library_selected().and_then(|(path, item)| {
                self.library
                    .assets
                    .iter()
                    .find(|a| {
                        a.kind == "image"
                            && a.item == Some(item)
                            && self.library.root.join(&a.document) == path
                    })
                    .cloned()
            }),
            WorkspaceKind::Canvas => self.canvas.library_path().and_then(|path| {
                self.library
                    .assets
                    .iter()
                    .find(|a| a.kind == "canvas" && self.library.root.join(&a.document) == path)
                    .cloned()
            }),
        };
        let mut open = None;
        ui.menu_button("Library", |ui| {
            for asset in &self.library.assets {
                if ui
                    .button(format!("{} · {}", asset.name, asset.kind))
                    .clicked()
                {
                    open = Some(asset.clone());
                    ui.close();
                }
            }
            if self.library.assets.is_empty() {
                ui.label("Import images or create a canvas to begin.");
            }
        });
        if let Some(asset) = open {
            self.open_asset(&asset, ui.ctx());
        }
        ui.add_enabled_ui(!self.library.busy, |ui| {
            if let Some(asset) = &selected {
                if asset.kind == "image"
                    && ui.button("Place on canvas").clicked()
                    && let Some(path) = self.canvas.library_path()
                {
                    self.library.send(Request::Place(asset.id, path));
                }
                if ui.button("Independent copy").clicked() {
                    self.library.send(Request::Copy(asset.id));
                }
            }
            if self.active == WorkspaceKind::Canvas
                && let Some((layer, asset, path)) = self.canvas.library_selected()
                && ui.button("Edit image").clicked()
            {
                if let Some(id) = asset {
                    if let Some(asset) = self.library.assets.iter().find(|a| a.id == id).cloned() {
                        self.open_asset(&asset, ui.ctx());
                    }
                } else if let Some(canvas) = self.canvas.library_path() {
                    self.library.send(Request::Adopt(layer, path, canvas));
                }
            }
        });
        if !self.library.status.is_empty() {
            ui.label(&self.library.status);
        }
    }
}
