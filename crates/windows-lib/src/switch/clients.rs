use crate::icon::set_icon;
use core_lib::{ClientData, ClientId};
use relm4::adw::gtk;
use relm4::adw::prelude::*;
use relm4::gtk::pango;
use relm4::prelude::*;

/// Clients component for `FlowBox` parent (non-workspace mode)
/// Shows clients in a flow layout without positioning
#[derive(Debug)]
pub struct Clients {
    active: bool,
    data: ClientData,
    pub id: ClientId,
    preview_size: (i32, i32),
    workspace_label: String,
    snapshot_session: crate::preview::SnapshotSession,
    snapshot_cache: crate::preview::SnapshotCache,
}

#[derive(Debug)]
pub enum ClientsInput {
    SetActive(bool),
}

#[derive(Debug)]
pub struct ClientsInit {
    pub data: ClientData,
    pub id: ClientId,
    pub preview_size: (i32, i32),
    pub workspace_label: String,
    pub snapshot_session: crate::preview::SnapshotSession,
    pub snapshot_cache: crate::preview::SnapshotCache,
}

#[derive(Debug)]
pub enum ClientsOutput {}

#[relm4::factory(pub)]
impl FactoryComponent for Clients {
    type Init = ClientsInit;
    type Input = ClientsInput;
    type Output = ClientsOutput;
    type CommandOutput = ();
    type ParentWidget = gtk::FlowBox;

    view! {
        gtk::FlowBoxChild {
            gtk::Button {
                #[watch]
                set_css_classes: if self.active { &["active", "client"] } else { &["client"] },
                set_cursor_from_name: Some("pointer"),
                set_width_request: self.preview_size.0,
                set_height_request: self.preview_size.1,
                gtk::Frame {
                    #[wrap(Some)]
                    set_label_widget = &gtk::Box {
                        set_css_classes: &["client-title"],
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 6,
                        set_width_request: (self.preview_size.0 - 12).max(1),
                        set_margin_start: 6,
                        #[name(title_icon)]
                        gtk::Image {
                            set_css_classes: &["client-title-icon"],
                            set_pixel_size: 16,
                            set_valign: gtk::Align::Center,
                        },
                        gtk::Label {
                            set_css_classes: &["client-title-text"],
                            set_overflow: gtk::Overflow::Visible,
                            set_max_width_chars: 1,
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_ellipsize: pango::EllipsizeMode::End,
                            set_label: if self.data.title.trim().is_empty() {
                                &self.data.class
                            } else {
                                &self.data.title
                            },
                        },
                    },
                    set_label_align: 0.5,
                    gtk::Overlay {
                        set_css_classes: &["client-thumbnail"],
                        set_size_request: (self.preview_size.0, self.preview_size.1),
                        gtk::DrawingArea {
                            set_content_width: self.preview_size.0,
                            set_content_height: self.preview_size.1,
                        },
                        #[name(preview)]
                        add_overlay = &gtk::Picture {
                            set_css_classes: &["client-preview"],
                            set_can_shrink: true,
                            set_content_fit: gtk::ContentFit::Contain,
                            set_can_target: false,
                            set_hexpand: true,
                            set_vexpand: true,
                        },
                        add_overlay = &gtk::Label {
                            set_css_classes: &["client-workspace"],
                            set_label: &self.workspace_label,
                            set_tooltip_text: Some(&format!("Workspace {}", self.workspace_label)),
                            set_halign: gtk::Align::End,
                            set_valign: gtk::Align::End,
                            set_margin_end: 6,
                            set_margin_bottom: 6,
                            set_ellipsize: pango::EllipsizeMode::End,
                            set_max_width_chars: 10,
                        },
                    }
                }
            }
        }
    }

    fn init_model(init: Self::Init, _index: &DynamicIndex, _sender: FactorySender<Self>) -> Self {
        // trace!("self {}x{}", init.data.width, init.data.height);
        Self {
            active: false,
            data: init.data,
            id: init.id,
            preview_size: init.preview_size,
            workspace_label: init.workspace_label,
            snapshot_session: init.snapshot_session,
            snapshot_cache: init.snapshot_cache,
        }
    }

    fn init_widgets(
        &mut self,
        _index: &DynamicIndex,
        root: Self::Root,
        _returned_widget: &<Self::ParentWidget as relm4::factory::FactoryView>::ReturnedWidget,
        _sender: FactorySender<Self>,
    ) -> Self::Widgets {
        let widgets = view_output!();

        set_icon(&self.data.class, self.data.pid, &widgets.title_icon);

        crate::preview::attach(
            &widgets.preview,
            self.id,
            self.data.pid,
            self.preview_size,
            self.snapshot_session.clone(),
            self.snapshot_cache.clone(),
        );

        widgets
    }

    fn update(&mut self, message: Self::Input, _sender: FactorySender<Self>) {
        match message {
            ClientsInput::SetActive(active) => {
                self.active = active;
            }
        }
    }
}
