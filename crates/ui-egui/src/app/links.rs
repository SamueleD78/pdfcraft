use super::*;
#[cfg(not(target_arch = "wasm32"))]
use sha2::{Digest, Sha256};

impl PdfCraftApp {
    /// A remembered decision applies only to these exact source bytes at their original
    /// absolute location, and to this one target path. File I/O is never performed here.
    #[cfg(not(target_arch = "wasm32"))]
    fn local_link_trust_key(&self, pending: &PendingRemoteLink) -> Option<crate::state::TrustedPdfLink> {
        const MAX_HASH_BYTES: usize = 32 * 1024 * 1024;
        let doc = self.session.get(pending.source)?;
        let source = doc.path.as_deref()?;
        let target = pending.path.to_str()?;
        if source.len() > 4096 || target.len() > 4096 || doc.bytes.len() > MAX_HASH_BYTES
            || !std::path::Path::new(source).is_absolute() || !pending.path.is_absolute()
        {
            return None;
        }
        let digest = Sha256::digest(doc.bytes.as_slice());
        Some(crate::state::TrustedPdfLink {
            source: source.to_string(),
            source_sha256: digest.iter().map(|byte| format!("{byte:02x}")).collect(),
            target: target.to_string(),
        })
    }

    /// Cheap eligibility check used by the dialog without hashing the PDF every frame.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn can_remember_local_link(&self, pending: &PendingRemoteLink) -> bool {
        const MAX_HASH_BYTES: usize = 32 * 1024 * 1024;
        self.session.get(pending.source).is_some_and(|doc| {
            doc.path.as_deref().is_some_and(|path| path.len() <= 4096 && std::path::Path::new(path).is_absolute())
                && pending.path.to_str().is_some_and(|target| target.len() <= 4096 && pending.path.is_absolute())
                && doc.bytes.len() <= MAX_HASH_BYTES
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn link_is_trusted(&self, pending: &PendingRemoteLink) -> bool {
        self.local_link_trust_key(pending).is_some_and(|key| self.trusted_pdf_links.contains(&key))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn remember_local_link(&mut self, pending: &PendingRemoteLink) {
        const MAX_SAVED: usize = 128;
        if let Some(key) = self.local_link_trust_key(pending) {
            // A changed source PDF replaces its prior decision for this same target.
            self.trusted_pdf_links.retain(|item| item.source != key.source || item.target != key.target);
            if self.trusted_pdf_links.len() >= MAX_SAVED {
                self.trusted_pdf_links.remove(0);
            }
            self.trusted_pdf_links.push(key);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn open_remote_link(&mut self, source: DocId, file: String, destination: pdfcraft_render::RemoteDestination) {
        if self.pending_remote_link.is_some() || self.pending_link.is_some() {
            return;
        }
        let Some(source_doc) = self.session.get(source) else { return };
        let source_path = source_doc.path.clone();
        let path = match pdfcraft_engine::resolve_remote_document_path(source_path.as_deref().map(std::path::Path::new), &file) {
            Ok(path) => path,
            Err(e) => {
                self.notify_fmt("Couldn't follow linked PDF: {reason}", &[("reason", &e.to_string())]);
                return;
            }
        };
        // Both relative and absolute paths need consent, unless the user has
        // explicitly remembered this source/target pair.
        let pending = PendingRemoteLink { source, path, destination };
        if self.link_is_trusted(&pending) {
            self.open_resolved_remote_link(pending.path, pending.destination);
        } else {
            self.pending_remote_link = Some(pending);
        }
    }

    /// Bound script-initiated opens so a single PDF cannot flood the UI.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn queue_script_remote_document(&mut self, pending: PendingRemoteLink) {
        const MAX_QUEUED: usize = 16;
        if self.pending_script_documents.len() >= MAX_QUEUED {
            self.notify_tr("Too many local PDF requests; additional requests were not queued.");
            return;
        }
        self.pending_script_documents.push_back(pending);
        self.next_script_remote_document();
    }

    /// Deliver each queued request to the existing local-PDF permission dialog.
    #[cfg(not(target_arch = "wasm32"))]
    fn next_script_remote_document(&mut self) {
        if self.pending_remote_link.is_some() || self.pending_link.is_some() {
            return;
        }
        while let Some(pending) = self.pending_script_documents.pop_front() {
            if self.session.get(pending.source).is_some() {
                if self.link_is_trusted(&pending) {
                    self.open_resolved_remote_link(pending.path, pending.destination);
                    continue;
                }
                self.pending_remote_link = Some(pending);
                break;
            }
        }
    }

    /// Take a consent decision exactly once; closing the source invalidates it.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn resolve_pending_remote_link(&mut self, open: bool) {
        let remember = std::mem::take(&mut self.remember_remote_link);
        if let Some(pending) = self.pending_remote_link.take()
            && open
            && self.session.get(pending.source).is_some()
        {
            let accepted = self.open_resolved_remote_link(pending.path.clone(), pending.destination.clone());
            if accepted && remember {
                self.remember_local_link(&pending);
            }
        }
        self.next_script_remote_document();
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open_resolved_remote_link(&mut self, path: std::path::PathBuf, destination: pdfcraft_render::RemoteDestination) -> bool {
        let path = path.to_string_lossy().into_owned();
        let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
        let stamp = file_watch::Stamp::at(&path);
        self.startup_superseded = true;
        let result = self.schedule_open(OpenRequest {
            name: name.clone(),
            path: Some(path.clone()),
            stamp,
            input: open_worker::Input::RemotePath(std::path::PathBuf::from(path)),
            password: None,
            activate: true,
            after_open: Some(destination),
            recovered: None,
            then: None,
        });
        match result {
            Ok(_) => true,
            Err(error) => {
                self.notify_fmt("Couldn't follow linked PDF: {reason}", &[("reason", &error)]);
                false
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn open_remote_link(&mut self, _source: DocId, _file: String, _destination: pdfcraft_render::RemoteDestination) {
        self.notify_tr("Local cross-PDF links require the desktop app");
    }

    #[cfg(target_arch = "wasm32")]
    pub fn resolve_pending_remote_link(&mut self, _open: bool) {
        self.pending_remote_link = None;
    }

    pub(super) fn apply_remote_destination(&mut self, destination: pdfcraft_render::RemoteDestination) {
        let Some(index) = self.views.len().checked_sub(1) else { return };
        let Some(doc) = self.session.get(self.views[index].id) else { return };
        let warning = match destination {
            pdfcraft_render::RemoteDestination::Default => None,
            pdfcraft_render::RemoteDestination::Named(_) => Some("The linked PDF opened, but its named destination was not applied."),
            pdfcraft_render::RemoteDestination::Page { page, view } if page < doc.info.pages.len() => {
                self.views[index].go_to_dest(page, view, &doc.info);
                None
            }
            pdfcraft_render::RemoteDestination::Page { .. } => Some("The linked PDF opened, but its page destination was out of range."),
        };
        if let Some(warning) = warning {
            self.notify_tr(warning);
        }
    }

    /// Open a web link in the system browser (a new tab on the web). Only for PdfCraft's own
    /// links; an address that came from a document goes through [`Self::request_document_url`].
    pub fn open_url(&mut self, url: &str) {
        if let Some(ctx) = &self.ctx {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
        self.last_opened_url = Some(url.to_string());
    }

    /// A document asks to open `url` (a link, a button's URI action or `app.launchURL`). Web and
    /// email addresses wait for the user to allow them; anything else is refused with a notice
    /// (#90, #91). While one request is waiting, further ones are dropped, so a script can't
    /// queue up a stream of dialogs.
    pub fn request_document_url(&mut self, url: &str, origin: LinkOrigin) {
        match pdfcraft_engine::links::document_url(url) {
            Ok(url) => {
                if self.pending_link.is_none() && self.pending_remote_link.is_none() {
                    self.pending_link = Some(PendingLink { url, origin });
                }
            }
            Err(e) => {
                use pdfcraft_engine::links::BlockedLink;
                let template = if matches!(e, BlockedLink::MailFile | BlockedLink::MailEncodedWord) {
                    "{who} in this document tried to open an address PdfCraft won't open: {e}."
                } else {
                    "{who} in this document tried to open an address PdfCraft won't open: {e}. Only web (http, https) and email (mailto) links open from documents."
                };
                self.notify_fmt(template, &[("who", tl!(origin.noun())), ("e", &e.to_string())]);
            }
        }
    }

    /// The user's answer to [`Self::pending_link`]: open it, or drop it.
    pub fn resolve_pending_link(&mut self, open: bool) {
        if let Some(p) = self.pending_link.take()
            && open
        {
            self.open_url(&p.url);
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.next_script_remote_document();
    }
}
