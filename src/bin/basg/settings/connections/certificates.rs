use super::*;
use gpui_kit::PathPromptOptions;

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

#[derive(Clone, Copy)]
pub(super) enum Field {
    Ca,
    Certificate,
    Key,
}

impl Field {
    pub(super) const ALL: [Self; 3] = [Self::Ca, Self::Certificate, Self::Key];

    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Ca => "connection-tls-ca",
            Self::Certificate => "connection-tls-certificate",
            Self::Key => "connection-tls-key",
        }
    }

    pub(super) fn label(self) -> &'static str {
        self.section().title()
    }

    pub(super) fn section(self) -> Section {
        match self {
            Self::Ca => Section::heading("connections-tls-ca"),
            Self::Certificate => Section::heading("connections-tls-certificate"),
            Self::Key => Section::heading("connections-tls-key"),
        }
    }

    pub(super) fn picker_label(self) -> &'static str {
        match self {
            Self::Ca => crate::i18n::tr!("connections-pick-ca"),
            Self::Certificate => crate::i18n::tr!("connections-pick-certificate"),
            Self::Key => crate::i18n::tr!("connections-pick-key"),
        }
    }

    pub(super) fn input(self, page: &Page) -> &Entity<InputState> {
        match self {
            Self::Ca => &page.tls_ca,
            Self::Certificate => &page.tls_certificate,
            Self::Key => &page.tls_key,
        }
    }
}

impl Page {
    pub(super) fn choose_certificate(
        &mut self,
        field: Field,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.certificate_picker_open
            || self.editor.is_none()
            || self.transport != Transport::Mtls
        {
            return;
        }
        self.certificate_picker_open = true;
        let generation = self.certificate_editor_generation;
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(field.picker_label().into()),
        });
        // Keep the dialog busy until its response, even if the editor is dismissed.
        // A generation also rejects switching away and back to the same transport.
        cx.spawn_in(window, async move |page, cx| {
            let result = paths.await;
            let _ = page.update_in(cx, |this, window, cx| {
                this.certificate_picker_open = false;
                cx.notify();
                if generation != this.certificate_editor_generation
                    || this.editor.is_none()
                    || this.transport != Transport::Mtls
                {
                    return;
                }
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first() {
                            if let Some(path) = path.to_str() {
                                field.input(this).update(cx, |input, cx| {
                                    input.set_value(path.to_owned(), window, cx);
                                });
                            } else {
                                this.feedback = Feedback::Error(
                                    crate::i18n::tr!("connections-pick-path-encoding").into(),
                                );
                            }
                        }
                    }
                    Ok(Ok(None)) => {}
                    _ => {
                        this.feedback =
                            Feedback::Error(crate::i18n::tr!("connections-pick-error").into());
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }
}
