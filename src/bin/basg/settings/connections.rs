use std::collections::HashMap;

use bas::config::ConnectionProfile;
use gpui_kit::{
    App, Context, Entity, FocusHandle, KeyBinding, ScrollHandle, Subscription, Window, actions,
    component::input::*, prelude::*,
};

use crate::connections::{self, Profiles};

mod certificates;
mod check;
mod discovery;
mod transport;
mod view;
use super::search::{Match, Query, Section};
use transport::Transport;

const NAME: Section = Section::heading("connections-name");
const NOTE: Section = Section::heading("connections-note");
const TYPE: Section = Section::heading("connections-type");
const SERVER_NAME: Section = Section::heading("connections-tls-server-name");
const LIST: Section = Section::heading("connections-list");
const DISCOVERY: Section = Section::new("connections-discover", "connections-discovery-scope");

pub(super) fn search(query: &Query) -> Option<Match> {
    [NAME, NOTE, TYPE, SERVER_NAME, LIST, DISCOVERY]
        .into_iter()
        .chain(
            Transport::AVAILABLE
                .iter()
                .map(|kind| kind.address_section()),
        )
        .chain(certificates::Field::ALL.map(|field| field.section()))
        .filter_map(|section| section.score_under(query, &[super::Category::Connections.section()]))
        .chain(
            Transport::AVAILABLE
                .iter()
                .filter_map(|kind| query.score(kind.label(), "")),
        )
        .max()
}

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

actions!(connection_editor, [CancelEditing, AddConnection]);

pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", CancelEditing, Some("BasConnectionEditor")),
        KeyBinding::new("escape", CancelEditing, Some("BasConnections")),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-n"
            } else {
                "ctrl-n"
            },
            AddConnection,
            Some("BasConnections"),
        ),
    ]);
}

enum Editor {
    Add,
    Edit(ConnectionProfile),
}

#[derive(Default)]
enum Feedback {
    #[default]
    Idle,
    Saved,
    Selected,
    Removed,
    Error(String),
}

pub(super) struct Page {
    profiles: Entity<Profiles>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    scroll_to: Option<usize>,
    name: Entity<InputState>,
    note: Entity<InputState>,
    target: Entity<InputState>,
    tls_ca: Entity<InputState>,
    tls_certificate: Entity<InputState>,
    tls_key: Entity<InputState>,
    tls_server_name: Entity<InputState>,
    certificate_picker_open: bool,
    certificate_editor_generation: u64,
    transport: Transport,
    addresses: HashMap<Transport, String>,
    editor: Option<Editor>,
    discovery: Option<discovery::State>,
    scan: Option<gpui_kit::Task<()>>,
    scan_generation: u64,
    checks: HashMap<String, check::Entry>,
    check_generation: u64,
    feedback: Feedback,
    _subscriptions: Vec<Subscription>,
}

impl Page {
    pub(super) fn focus_page(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
    }

    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let profiles = connections::shared(cx);
        let name = cx.new(|cx| InputState::new(window, cx));
        let note = cx.new(|cx| InputState::new(window, cx));
        let target = cx.new(|cx| InputState::new(window, cx));
        let tls_ca = cx.new(|cx| InputState::new(window, cx));
        let tls_certificate = cx.new(|cx| InputState::new(window, cx));
        let tls_key = cx.new(|cx| InputState::new(window, cx));
        let tls_server_name = cx.new(|cx| InputState::new(window, cx));
        let mut subscriptions = vec![cx.observe(&profiles, |this: &mut Self, _, cx| {
            this.prune_checks(cx);
            cx.notify();
        })];
        for input in [
            &name,
            &note,
            &target,
            &tls_ca,
            &tls_certificate,
            &tls_key,
            &tls_server_name,
        ] {
            subscriptions.push(
                cx.subscribe_in(input, window, |this, _, event, window, cx| match event {
                    InputEvent::PressEnter { .. } if this.editor.is_some() => this.save(window, cx),
                    InputEvent::Change if matches!(this.feedback, Feedback::Error(_)) => {
                        this.feedback = Feedback::Idle;
                        cx.notify();
                    }
                    _ => {}
                }),
            );
        }
        Self {
            profiles,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            scroll_to: None,
            name,
            note,
            target,
            tls_ca,
            tls_certificate,
            tls_key,
            tls_server_name,
            certificate_picker_open: false,
            certificate_editor_generation: 0,
            transport: Transport::Local,
            addresses: HashMap::new(),
            editor: None,
            discovery: None,
            scan: None,
            scan_generation: 0,
            checks: HashMap::new(),
            check_generation: 0,
            feedback: Feedback::Idle,
            _subscriptions: subscriptions,
        }
    }

    pub(super) fn status(&self, cx: &App) -> String {
        if let Some(error) = self.error(cx) {
            return error;
        }
        match self.feedback {
            Feedback::Saved => crate::i18n::tr!("settings-saved").into(),
            Feedback::Selected => crate::i18n::tr!("connections-selected").into(),
            Feedback::Removed => crate::i18n::tr!("connections-removed").into(),
            Feedback::Idle | Feedback::Error(_) => String::new(),
        }
    }

    fn error(&self, cx: &App) -> Option<String> {
        match &self.feedback {
            Feedback::Error(error) => Some(error.clone()),
            _ => self.profiles.read(cx).error.clone(),
        }
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() {
            return;
        }
        self.dismiss_discovery();
        self.editor = Some(Editor::Add);
        self.certificate_editor_generation += 1;
        self.scroll_to = Some(0);
        self.transport = Transport::Local;
        self.addresses.clear();
        self.name
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.note
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.target
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.set_tls(None, window, cx);
        self.feedback = Feedback::Idle;
        self.name.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_discovery();
        self.editor = None;
        self.certificate_editor_generation += 1;
        self.feedback = Feedback::Idle;
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn edit(&mut self, profile: &ConnectionProfile, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() {
            return;
        }
        self.dismiss_discovery();
        if !self.profiles.read(cx).entries().contains(profile) {
            self.feedback = Feedback::Error(crate::i18n::tr!("connections-changed").into());
            cx.notify();
            return;
        }
        self.editor = Some(Editor::Edit(profile.clone()));
        self.certificate_editor_generation += 1;
        self.scroll_to = self
            .profiles
            .read(cx)
            .entries()
            .iter()
            .position(|entry| entry.id == profile.id);
        self.transport = Transport::from_target(&profile.target);
        self.addresses.clear();
        self.name.update(cx, |input, cx| {
            input.set_value(profile.name.clone(), window, cx)
        });
        self.note.update(cx, |input, cx| {
            input.set_value(profile.note.clone().unwrap_or_default(), window, cx)
        });
        self.target.update(cx, |input, cx| {
            input.set_value(Transport::address(&profile.target), window, cx);
            input.set_placeholder(self.transport.placeholder(), window, cx);
        });
        self.set_tls(profile.tls.as_ref(), window, cx);
        self.feedback = Feedback::Idle;
        self.name.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn set_tls(
        &mut self,
        tls: Option<&bas::service::tls::ClientTls>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (input, value) in [
            (
                &self.tls_ca,
                tls.map(|tls| tls.ca.to_string_lossy().into_owned()),
            ),
            (
                &self.tls_certificate,
                tls.map(|tls| tls.certificate.to_string_lossy().into_owned()),
            ),
            (
                &self.tls_key,
                tls.map(|tls| tls.private_key.to_string_lossy().into_owned()),
            ),
            (
                &self.tls_server_name,
                tls.and_then(|tls| tls.server_name.clone()),
            ),
        ] {
            input.update(cx, |input, cx| {
                input.set_value(value.unwrap_or_default(), window, cx)
            });
        }
    }

    fn select_transport(
        &mut self,
        transport: Transport,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if transport != self.transport {
            self.certificate_editor_generation += 1;
        }
        self.addresses
            .insert(self.transport, self.target.read(cx).value().to_string());
        self.transport = transport;
        let address = self.addresses.get(&transport).cloned().unwrap_or_default();
        self.target.update(cx, |input, cx| {
            input.set_value(address, window, cx);
            input.set_placeholder(transport.placeholder(), window, cx);
            if transport != Transport::Local {
                input.focus(window, cx);
            }
        });
        self.feedback = Feedback::Idle;
        cx.notify();
    }

    fn choose(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.editor.is_some() {
            return;
        }
        match self
            .profiles
            .update(cx, |profiles, cx| profiles.select(id, cx))
        {
            Ok(target) => {
                connections::manage(target, cx);
                self.feedback = Feedback::Selected;
            }
            Err(error) => self.feedback = Feedback::Error(error),
        }
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &self.editor else { return };
        let id = match editor {
            Editor::Add => uuid::Uuid::new_v4().to_string(),
            Editor::Edit(original) => {
                if !self.profiles.read(cx).entries().contains(original) {
                    self.feedback = Feedback::Error(crate::i18n::tr!("connections-changed").into());
                    cx.notify();
                    return;
                }
                original.id.clone()
            }
        };
        let name = self.name.read(cx).value().trim().to_owned();
        if name.is_empty() {
            self.feedback = Feedback::Error(crate::i18n::tr!("connections-name-required").into());
            self.name.update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
            return;
        }
        let target = match self.transport.target(&self.target.read(cx).value()) {
            Ok(target) => target,
            Err(error) => {
                self.feedback = Feedback::Error(error);
                self.target.update(cx, |input, cx| input.focus(window, cx));
                cx.notify();
                return;
            }
        };
        let note = self.note.read(cx).value().trim().to_owned();
        let tls = (self.transport == Transport::Mtls).then(|| {
            let server_name = self.tls_server_name.read(cx).value().trim().to_owned();
            bas::service::tls::ClientTls {
                ca: self.tls_ca.read(cx).value().as_str().into(),
                certificate: self.tls_certificate.read(cx).value().as_str().into(),
                private_key: self.tls_key.read(cx).value().as_str().into(),
                server_name: (!server_name.is_empty()).then_some(server_name),
            }
        });
        let profile = ConnectionProfile {
            id,
            name,
            note: (!note.is_empty()).then_some(note),
            target,
            tls,
        };
        match self
            .profiles
            .update(cx, |profiles, cx| profiles.put(profile, cx))
        {
            Ok(_) => {
                self.editor = None;
                self.focus.focus(window, cx);
                self.feedback = Feedback::Saved;
            }
            Err(error) => self.feedback = Feedback::Error(error),
        }
        cx.notify();
    }

    fn request_remove(&mut self, profile: ConnectionProfile, cx: &mut Context<Self>) {
        if self.editor.is_some() || self.profiles.read(cx).entries().len() <= 1 {
            return;
        }
        let owner = cx.entity().downgrade();
        let request = crate::application::Confirmation {
            title: crate::i18n::tr!("connections-remove").into(),
            message: crate::i18n::tr!("connections-remove-confirm", name = profile.name.clone()),
            confirm_label: crate::i18n::tr!("connections-remove").into(),
            destructive: true,
        };
        if let Err(error) = crate::application::open_confirmation(
            request,
            move |cx| {
                let _ = owner.update(cx, |this, cx| this.remove(&profile, cx));
            },
            cx,
        ) {
            self.feedback = Feedback::Error(error.to_string());
            cx.notify();
        }
    }

    fn remove(&mut self, profile: &ConnectionProfile, cx: &mut Context<Self>) {
        if self.editor.is_some() || !self.profiles.read(cx).entries().contains(profile) {
            self.feedback = Feedback::Error(crate::i18n::tr!("connections-changed").into());
            cx.notify();
            return;
        }
        match self
            .profiles
            .update(cx, |profiles, cx| profiles.remove(&profile.id, cx))
        {
            Ok(target) => {
                if connections::current(cx).as_ref() == Some(&profile.options()) {
                    connections::manage(target, cx);
                }
                self.feedback = Feedback::Removed;
            }
            Err(error) => self.feedback = Feedback::Error(error),
        }
        cx.notify();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        if self.editor.is_some() {
            return;
        }
        self.profiles.update(cx, |profiles, cx| profiles.reload(cx));
        self.feedback = Feedback::Idle;
        cx.notify();
    }
}
