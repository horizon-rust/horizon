//! The Builds page: every instance in one wide list instead of the sidebar's narrow column, with
//! the version each one plays and whether that version is prepared. A row opens that build; its
//! actions (edit, copy, export, launch, shortcut, folder, delete) stay on the build's own page.
use super::*;
use horizon_launcher::instances::Kind as InstanceKind;

impl Launcher {
    pub(super) fn builds_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        self.load_packs();
        let title = div()
            .flex()
            .items_start()
            .justify_between()
            .child(page_header(
                tr("Builds"),
                tr("Every build in one list: the version it plays, and whether that version is ready."),
                &t,
            ))
            .child(button("new-build", tr("New build"), Kind::Secondary, &t).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.open_instance_dialog(None);
                    cx.notify();
                },
            )));
        let chosen = self.selected_instance().map(|i| i.id.clone());
        let list = self.instances.list.clone();
        let mut rows: Vec<AnyElement> = Vec::with_capacity(list.len());
        for instance in list {
            let open = chosen.as_ref() == Some(&instance.id);
            let tag = self.version_of(&instance).cloned();
            let prepared = tag.as_deref().is_some_and(backend::is_prepared);
            let sub = match &instance.kind {
                // The states the sidebar's row had, in the same order: a Java instance first.
                InstanceKind::Default | InstanceKind::Own => {
                    match (&instance.java, &tag, &instance.version) {
                        (Some(java), _, _) => instances_page::java_line(java),
                        (None, None, Some(own)) => {
                            trf("{version} is not installed", &[("version", own)])
                        }
                        (None, None, None) => tr("No version installed").to_string(),
                        (None, Some(_), _) if !prepared => tr("Needs preparing").to_string(),
                        (None, Some(tag), Some(_)) => format!("{} · {tag}", tr("Ready")),
                        (None, Some(_), None) => tr("Ready").to_string(),
                    }
                }
                InstanceKind::Port => {
                    let slug = &instance.id["port:".len()..];
                    self.packs
                        .iter()
                        .find(|p| p.slug == slug)
                        .map_or_else(|| tr("Modpack").to_string(), |p| p.summary())
                }
                InstanceKind::Pack => tr("From Horizon's registry").to_string(),
            };
            let icon = self.instance_icon(&instance, 40.);
            // A build that plays Horizon's game is ready or needs preparing; a pack or a Java
            // build has neither.
            let dot = div().size(px(6.)).flex_none().bg(match instance.kind {
                InstanceKind::Default | InstanceKind::Own if instance.java.is_some() => t.muted,
                InstanceKind::Default | InstanceKind::Own if prepared => t.success,
                InstanceKind::Default | InstanceKind::Own => t.warn,
                InstanceKind::Port | InstanceKind::Pack => t.muted,
            });
            let clicked = instance.clone();
            rows.push(
                slot_row(
                    Some(div().size(px(40.)).child(icon)),
                    div()
                        .flex()
                        .items_center()
                        .gap(px(9.))
                        .child(dot)
                        .child(instance.name.clone())
                        .into_any_element(),
                    Some(
                        text(theme::MICRO)
                            .text_color(t.muted)
                            .child(sub)
                            .into_any_element(),
                    ),
                    open.then(|| {
                        div()
                            .w(px(80.))
                            .flex()
                            .justify_end()
                            .child(caps("In use").text_color(t.accent))
                            .into_any_element()
                    }),
                    &t,
                )
                .id(SharedString::from(format!("build-{}", instance.id)))
                .cursor_pointer()
                .hover(move |s| s.bg(t.raised))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    match clicked.kind {
                        InstanceKind::Default | InstanceKind::Own => {
                            this.select_instance(&clicked);
                            this.go(Page::Play);
                        }
                        InstanceKind::Port => {
                            this.show_modpack(Some(clicked.id["port:".len()..].into()))
                        }
                        InstanceKind::Pack => this.show_modpack(None),
                    }
                    cx.notify();
                }))
                .into_any_element(),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(title)
            .children(rows)
    }
}
