use crate::data::{SortConfig, collect_data};
use crate::next::{find_next_client, find_next_workspace};
use crate::shared::{Workspaces, WorkspacesInit, WorkspacesInput};
use crate::switch::clients::{Clients, ClientsInit};
use core_lib::{Active, ByFirst, Direction, HyprlandData, SWITCH_NAMESPACE};
use exec_lib::switch::{switch_client, switch_workspace};
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};
use regex::Regex;
use relm4::adw::glib::ControlFlow;
use relm4::adw::gtk;
use relm4::adw::gtk::glib;
use relm4::adw::prelude::*;
use relm4::gtk::gdk::{Device, Key, ModifierType};
use relm4::gtk::{EventControllerKey, Orientation, SelectionMode};
use relm4::prelude::*;
use std::time::Duration;
use tracing::{debug, error, trace};

const KILL_TIMEOUT: Duration = Duration::from_millis(200);
const SHOW_DELAY: Duration = Duration::from_millis(150);

#[derive(Debug)]
pub struct SwitchRoot {
    general: config_lib::WindowsGeneral,
    switch: config_lib::Switch,
    open: bool,
    show_generation: u64,
    data: SwitchData,
    // gtk
    window: gtk::ApplicationWindow,
    controller: Option<gtk::EventController>,
    keyboard: Option<(Device, glib::SignalHandlerId)>,
    /// Regex for removing HTML tags from strings
    remove_html: Regex,
    /// Factory for workspace mode (workspaces)
    items: FactoryVecDeque<Workspaces>,
    /// Factory for non-workspace mode (clients)
    clients_only: FactoryVecDeque<Clients>,
}

#[derive(Debug)]
pub enum SwitchRootInput {
    SetSwitch(config_lib::Switch),
    SetGeneral(config_lib::WindowsGeneral),
    OpenSwitch(Direction),
    ShowSwitch(u64),
    CheckModifiers,
    Switch(Direction),
    CloseSwitch(bool),
    CloseCurrentItem,
    ReloadSwitch,
}

#[derive(Debug)]
pub struct SwitchRootInit {
    pub general: config_lib::WindowsGeneral,
    pub switch: config_lib::Switch,
}

#[derive(Debug)]
pub enum SwitchRootOutput {}

#[relm4::component(pub)]
impl SimpleComponent for SwitchRoot {
    type Init = SwitchRootInit;
    type Input = SwitchRootInput;
    type Output = SwitchRootOutput;

    view! {
        #[root]
        gtk::ApplicationWindow {
            set_css_classes: &["window", "switch"],
            set_decorated: false,
            set_default_size: (100, 100),
            match model.switch.switch_workspaces {
                true => {
                    #[local_ref]
                    itemsw -> gtk::FlowBox {
                        set_css_classes: &["monitor"],
                        set_selection_mode: SelectionMode::None,
                        set_orientation: Orientation::Horizontal,
                        #[watch]
                        set_max_children_per_line: u32::from(model.general.items_per_row),
                        #[watch]
                        set_min_children_per_line: u32::from(model.general.items_per_row),
                    }
                }
                false => {
                    #[local_ref]
                    clients_only_w -> gtk::FlowBox {
                        set_css_classes: &["monitor"],
                        set_selection_mode: SelectionMode::None,
                        set_orientation: Orientation::Horizontal,
                        #[watch]
                        set_max_children_per_line: u32::from(model.general.items_per_row),
                        #[watch]
                        set_min_children_per_line: u32::from(model.general.items_per_row),
                    }
                }
            }
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        trace!("Initializing SwitchRoot");

        let items: FactoryVecDeque<Workspaces> = FactoryVecDeque::builder()
            .launch(gtk::FlowBox::default())
            .detach();

        let clients_only: FactoryVecDeque<Clients> = FactoryVecDeque::builder()
            .launch(gtk::FlowBox::default())
            .detach();

        let keyboard = WidgetExt::display(&root)
            .default_seat()
            .and_then(|seat| seat.keyboard())
            .map(|keyboard| {
                let sender = sender.clone();
                let handler = keyboard.connect_modifier_state_notify(move |_| {
                    sender.input(SwitchRootInput::CheckModifiers);
                });
                (keyboard, handler)
            });
        let focus_sender = sender.clone();
        root.connect_is_active_notify(move |window| {
            if window.is_active() {
                focus_sender.input(SwitchRootInput::CheckModifiers);
            }
        });

        let model = Self {
            general: init.general,
            switch: init.switch,
            open: false,
            show_generation: 0,
            window: root.clone(),
            controller: None,
            keyboard,
            remove_html: Regex::new(r"<[^>]*>").expect("invalid regex"),
            data: SwitchData::default(),
            items,
            clients_only,
        };

        let itemsw: gtk::FlowBox = model.items.widget().clone();
        let clients_only_w: gtk::FlowBox = model.clients_only.widget().clone();
        let widgets = view_output!();

        let window = &root;
        window.init_layer_shell();
        window.set_namespace(Some(SWITCH_NAMESPACE));
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        sender
            .input_sender()
            .emit(SwitchRootInput::SetSwitch(model.switch.clone()));
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        trace!("switch::root::update: {message:?}");
        match message {
            SwitchRootInput::SetSwitch(switch) => {
                self.switch = switch;
                self.setup_keyboard_controller(&sender);
            }
            SwitchRootInput::SetGeneral(general) => {
                self.general = general;
                self.setup_keyboard_controller(&sender);
            }
            SwitchRootInput::OpenSwitch(direction) => {
                if self.open {
                    sender
                        .input_sender()
                        .emit(SwitchRootInput::Switch(direction));
                } else {
                    self.open = self.prepare_switch(direction);
                    if self.open {
                        self.show_generation = self.show_generation.wrapping_add(1);
                        if self.switch.modifier == config_lib::Modifier::None {
                            self.show_switch();
                        } else {
                            let generation = self.show_generation;
                            glib::timeout_add_local_once(SHOW_DELAY, move || {
                                sender
                                    .input_sender()
                                    .emit(SwitchRootInput::ShowSwitch(generation));
                            });
                        }
                    }
                }
            }
            SwitchRootInput::ShowSwitch(generation) => {
                // A quick release must not show the popup, and a timer from an
                // earlier Alt-Tab must not shorten the delay for the next one.
                if self.open && self.show_generation == generation {
                    self.show_switch();
                }
            }
            SwitchRootInput::CheckModifiers => {
                // Before keyboard focus arrives, the device state may belong
                // to the previously focused surface. Check only while active.
                if self.open
                    && self.window.is_active()
                    && let Some((keyboard, _)) = &self.keyboard
                    && !switch_modifier_is_held(self.switch.modifier, keyboard.modifier_state())
                {
                    trace!("Switch modifier is no longer held");
                    self.open = false;
                    self.close_switch(true);
                }
            }
            SwitchRootInput::Switch(direction) => {
                if self.open {
                    self.navigate(direction);
                } else {
                    trace!("not open");
                }
            }
            SwitchRootInput::CloseSwitch(do_switch) => {
                if self.open {
                    self.open = false;
                    self.close_switch(do_switch);
                } else {
                    trace!("not open");
                }
            }
            SwitchRootInput::CloseCurrentItem => {
                if self.open {
                    self.close_item();
                } else {
                    trace!("not open");
                }
                sender.input_sender().emit(SwitchRootInput::ReloadSwitch);
            }
            SwitchRootInput::ReloadSwitch => {
                if self.open {
                    self.reload_switch();
                } else {
                    trace!("not open");
                }
            }
        }
    }

    fn shutdown(&mut self, _widgets: &mut Self::Widgets, _output: relm4::Sender<Self::Output>) {
        if let Some((keyboard, handler)) = self.keyboard.take() {
            keyboard.disconnect(handler);
        }
    }
}

impl SwitchRoot {
    fn setup_keyboard_controller(&mut self, sender: &ComponentSender<Self>) {
        // TODO add a check in config check so these always succeed
        if let Some(k) = Key::from_name(self.switch.key.to_string()) {
            if let Some(kk) = Key::from_name(self.switch.kill_key.to_string()) {
                let key_controller = EventControllerKey::new();
                let sender_2 = sender.clone();
                key_controller.connect_key_pressed(move |_, key, _, _| {
                    trace!("Key pressed: {:?}", key);
                    handle_key(key, k, kk, &sender_2.clone())
                });
                // The compositor's release binding may be missed while this
                // layer surface owns the keyboard. Handle the release here too.
                let modifier = self.switch.modifier;
                let sender_2 = sender.clone();
                key_controller.connect_key_released(move |_, key, _, _| {
                    if is_switch_modifier(key, modifier) {
                        trace!("Switch modifier released: {:?}", key);
                        sender_2
                            .input_sender()
                            .emit(SwitchRootInput::CloseSwitch(true));
                    }
                });
                if let Some(controller) = self.controller.take() {
                    self.window.remove_controller(&controller);
                }
                self.window.add_controller(key_controller.clone());
                self.controller = Some(key_controller.upcast());
            } else {
                error!("Invalid kill key name: {}", self.switch.kill_key);
            }
        } else {
            error!("Invalid key name: {}", self.switch.key);
        }
    }

    fn prepare_switch(&mut self, direction: Direction) -> bool {
        let (hypr_data, active_prev) = match collect_data(&SortConfig {
            filter_current_monitor: self.switch.filter_by_current_monitor,
            filter_current_workspace: self.switch.filter_by_current_workspace,
            filter_same_class: self.switch.filter_by_same_class,
            sort_recent: true,
            exclude_workspaces: if self.switch.exclude_workspaces.is_empty() {
                None
            } else {
                Some(self.switch.exclude_workspaces.clone())
            },
        }) {
            Ok(data) => data,
            Err(e) => {
                error!("Failed to collect data: {}", e);
                return false;
            }
        };

        let active = if self.switch.switch_workspaces {
            find_next_workspace(
                direction,
                true,
                &hypr_data,
                active_prev,
                self.general.items_per_row,
            )
        } else {
            find_next_client(
                direction,
                true,
                &hypr_data,
                active_prev,
                self.general.items_per_row,
            )
        };
        self.data = SwitchData { active, hypr_data };
        true
    }

    fn show_switch(&mut self) {
        let hypr_data = self.data.hypr_data.clone();
        if self.switch.switch_workspaces {
            self.populate_workspace_mode(&hypr_data, self.general.scale, self.data.active);
        } else {
            self.populate_clients_only_mode(&hypr_data, self.general.scale, self.data.active);
        }

        trace!("Showing window {:?}", self.window.id());
        self.window.set_visible(true);
        self.window.grab_focus();
    }

    fn populate_workspace_mode(&mut self, hypr_data: &HyprlandData, scale: f64, active: Active) {
        let mut lock = self.items.guard();
        lock.clear();

        for (wid, workspace_data) in &hypr_data.workspaces {
            if !workspace_data.any_client_enabled {
                trace!("skipping workspace {} with no enabled clients", wid);
                continue;
            }
            // Get clients for this workspace
            let workspace_clients: Vec<_> = hypr_data
                .clients
                .iter()
                .filter(|(_, client)| client.workspace == *wid && client.enabled)
                .map(|(id, data)| (*id, data.clone()))
                .collect();

            let Some(monitor) = hypr_data.monitors.find_by_first(&workspace_data.monitor) else {
                error!(
                    "Workspace {} has invalid monitor {}",
                    wid, workspace_data.monitor
                );
                continue;
            };
            lock.push_back(WorkspacesInit {
                monitor_data: monitor.clone(),
                remove_html: self.remove_html.clone(),
                id: *wid,
                data: workspace_data.clone(),
                scale,
                clients: workspace_clients,
            });
        }
        drop(lock);

        // Set active workspace
        for (idx, item) in self.items.iter().enumerate() {
            if item.workspace_id == active.workspace {
                self.items.send(idx, WorkspacesInput::SetActive(true));
                break;
            }
        }
    }

    fn populate_clients_only_mode(&mut self, hypr_data: &HyprlandData, scale: f64, active: Active) {
        let mut lock = self.clients_only.guard();
        lock.clear();

        for (id, client) in &hypr_data.clients {
            if !client.enabled {
                continue;
            }
            let Some(monitor) = hypr_data.monitors.find_by_first(&client.monitor) else {
                error!("Client {} has invalid monitor {}", id, client.monitor);
                continue;
            };
            lock.push_back(ClientsInit {
                id: *id,
                scale,
                monitor_data: monitor.clone(),
                data: client.clone(),
                workspace_label: if client.workspace > 0 {
                    client.workspace.to_string()
                } else {
                    hypr_data
                        .workspaces
                        .find_by_first(&client.workspace)
                        .map_or_else(
                            || "?".to_string(),
                            |workspace| workspace.name.trim_start_matches("special:").to_string(),
                        )
                },
            });
        }
        drop(lock);

        // Set active client
        if let Some(active_id) = active.client {
            for (idx, item) in self.clients_only.iter().enumerate() {
                if item.id == active_id {
                    self.clients_only
                        .send(idx, crate::switch::clients::ClientsInput::SetActive(true));
                    break;
                }
            }
        }
    }

    fn navigate(&mut self, direction: Direction) {
        let new_active = if self.switch.switch_workspaces {
            find_next_workspace(
                direction,
                true,
                &self.data.hypr_data,
                self.data.active,
                self.general.items_per_row,
            )
        } else {
            find_next_client(
                direction,
                true,
                &self.data.hypr_data,
                self.data.active,
                self.general.items_per_row,
            )
        };

        let old_active = self.data.active;
        self.data.active = new_active;

        if self.switch.switch_workspaces {
            self.update_workspace_active(old_active, new_active);
        } else {
            self.update_clients_only_active(old_active, new_active);
        }
    }

    fn update_workspace_active(&self, old_active: Active, new_active: Active) {
        // Update workspace active state
        if old_active.workspace != new_active.workspace {
            for (idx, item) in self.items.iter().enumerate() {
                if item.workspace_id == old_active.workspace {
                    self.items.send(idx, WorkspacesInput::SetActive(false));
                }
                if item.workspace_id == new_active.workspace {
                    self.items.send(idx, WorkspacesInput::SetActive(true));
                    if let Some(cid) = new_active.client {
                        self.items.send(idx, WorkspacesInput::SetActiveClient(cid));
                    }
                }
            }
        }
    }

    fn update_clients_only_active(&self, old_active: Active, new_active: Active) {
        // Clear old active
        if let Some(old_id) = old_active.client {
            for (idx, item) in self.clients_only.iter().enumerate() {
                if item.id == old_id {
                    self.clients_only
                        .send(idx, crate::switch::clients::ClientsInput::SetActive(false));
                    break;
                }
            }
        }

        // Set new active
        if let Some(new_id) = new_active.client {
            for (idx, item) in self.clients_only.iter().enumerate() {
                if item.id == new_id {
                    self.clients_only
                        .send(idx, crate::switch::clients::ClientsInput::SetActive(true));
                    break;
                }
            }
        }
    }

    fn close_switch(&mut self, do_switch: bool) {
        trace!("Hiding window {:?}", self.window.id());
        self.window.set_visible(false);

        // Clear UI
        {
            let mut lock = self.items.guard();
            lock.clear();
        }
        {
            let mut lock = self.clients_only.guard();
            lock.clear();
        }

        if do_switch {
            if let Some(id) = self.data.active.client {
                debug!(
                    "Switching to client {}",
                    self.data
                        .hypr_data
                        .clients
                        .iter()
                        .find(|(cid, _)| *cid == id)
                        .map_or_else(|| "<Unknown>".to_string(), |(_, c)| c.title.clone())
                );
                // Defer execution to ensure window is hidden first
                glib::idle_add_local(move || {
                    if let Err(e) = switch_client(id) {
                        tracing::warn!("Failed to switch to client {id:?}: {e}");
                    }
                    ControlFlow::Break
                });
            } else {
                let id = self.data.active.workspace;
                debug!(
                    "Switching to workspace {}",
                    self.data
                        .hypr_data
                        .workspaces
                        .iter()
                        .find(|(wid, _)| *wid == id)
                        .map_or_else(|| "<Unknown>".to_string(), |(_, w)| w.name.clone())
                );
                glib::idle_add_local(move || {
                    if let Err(e) = switch_workspace(id) {
                        tracing::warn!("Failed to switch to workspace {id:?}: {e}");
                    }
                    ControlFlow::Break
                });
            }
        }
    }

    fn close_item(&self) {
        if self.switch.switch_workspaces {
            self.kill_workspace_clients();
        } else {
            self.kill_active_client();
        }
    }

    fn kill_active_client(&self) {
        if let Some(id) = self.data.active.client
            && let Err(e) = exec_lib::kill::kill_client_blocking(id, KILL_TIMEOUT)
        {
            // TODO: close on killed to let user close window themself
            tracing::warn!("Failed to kill client {id}: {e}");
        }
    }

    fn kill_workspace_clients(&self) {
        let workspace_id = self.data.active.workspace;
        debug!(
            "Killing all clients in workspace {}",
            self.data
                .hypr_data
                .workspaces
                .iter()
                .find(|(wid, _)| *wid == workspace_id)
                .map_or_else(|| workspace_id.to_string(), |(_, w)| w.name.clone())
        );

        let clients_to_kill: Vec<_> = self
            .data
            .hypr_data
            .clients
            .iter()
            .filter(|(_, client)| client.workspace == workspace_id)
            .map(|(id, _)| *id)
            .collect();

        for client_id in clients_to_kill {
            if let Err(e) = exec_lib::kill::kill_client_blocking(client_id, KILL_TIMEOUT) {
                // TODO: close on killed to let user close window themself
                tracing::warn!("Failed to kill client {client_id}: {e}");
            }
        }
    }

    fn reload_switch(&mut self) {
        let (hypr_data, _) = match collect_data(&SortConfig {
            filter_current_monitor: self.switch.filter_by_current_monitor,
            filter_current_workspace: self.switch.filter_by_current_workspace,
            filter_same_class: self.switch.filter_by_same_class,
            sort_recent: true,
            exclude_workspaces: if self.switch.exclude_workspaces.is_empty() {
                None
            } else {
                Some(self.switch.exclude_workspaces.clone())
            },
        }) {
            Ok(data) => data,
            Err(e) => {
                error!("Failed to collect data: {}", e);
                return;
            }
        };

        while match self.data.active {
            Active {
                client: Some(id), ..
            } => hypr_data.clients.find_by_first(&id).is_none(),
            Active { workspace: id, .. } => hypr_data.workspaces.find_by_first(&id).is_none(),
        } {
            self.data.active = if self.switch.switch_workspaces {
                find_next_workspace(
                    Direction::Right,
                    true,
                    &hypr_data,
                    self.data.active,
                    self.general.items_per_row,
                )
            } else {
                find_next_client(
                    Direction::Right,
                    true,
                    &hypr_data,
                    self.data.active,
                    self.general.items_per_row,
                )
            };
        }

        self.data = SwitchData {
            active: self.data.active,
            hypr_data: hypr_data.clone(),
        };

        if self.switch.switch_workspaces {
            self.populate_workspace_mode(&hypr_data, self.general.scale, self.data.active);
        } else {
            self.populate_clients_only_mode(&hypr_data, self.general.scale, self.data.active);
        }
    }
}

const fn switch_modifier_is_held(modifier: config_lib::Modifier, state: ModifierType) -> bool {
    match modifier {
        config_lib::Modifier::Alt => state.contains(ModifierType::ALT_MASK),
        config_lib::Modifier::Ctrl => state.contains(ModifierType::CONTROL_MASK),
        config_lib::Modifier::Super => state.contains(ModifierType::SUPER_MASK),
        config_lib::Modifier::None => true,
    }
}

const fn is_switch_modifier(key: Key, modifier: config_lib::Modifier) -> bool {
    match modifier {
        config_lib::Modifier::Alt => matches!(key, Key::Alt_L | Key::Alt_R),
        config_lib::Modifier::Ctrl => matches!(key, Key::Control_L | Key::Control_R),
        config_lib::Modifier::Super => matches!(key, Key::Super_L | Key::Super_R),
        config_lib::Modifier::None => false,
    }
}

fn handle_key(
    key: Key,
    s_key: Key,
    kill_key: Key,
    event_sender: &ComponentSender<SwitchRoot>,
) -> glib::Propagation {
    match key {
        Key::Escape => {
            event_sender
                .input_sender()
                .emit(SwitchRootInput::CloseSwitch(false));
            glib::Propagation::Stop
        }
        k if k == s_key || k == Key::l || k == Key::Right => {
            event_sender
                .input_sender()
                .emit(SwitchRootInput::Switch(Direction::Right));
            glib::Propagation::Stop
        }
        Key::ISO_Left_Tab | Key::grave | Key::dead_grave | Key::h | Key::Left => {
            event_sender
                .input_sender()
                .emit(SwitchRootInput::Switch(Direction::Left));
            glib::Propagation::Stop
        }
        Key::j | Key::Down => {
            event_sender
                .input_sender()
                .emit(SwitchRootInput::Switch(Direction::Down));
            glib::Propagation::Stop
        }
        Key::k | Key::Up => {
            event_sender
                .input_sender()
                .emit(SwitchRootInput::Switch(Direction::Up));
            glib::Propagation::Stop
        }
        k if k == kill_key || k == Key::Delete => {
            event_sender
                .input_sender()
                .emit(SwitchRootInput::CloseCurrentItem);
            glib::Propagation::Stop
        }
        _ => glib::Propagation::Proceed,
    }
}

#[derive(Debug)]
pub struct SwitchData {
    pub active: Active,
    pub hypr_data: HyprlandData,
}

impl Default for SwitchData {
    fn default() -> Self {
        Self {
            active: Active {
                client: None,
                workspace: -1,
                monitor: -1,
            },
            hypr_data: HyprlandData::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_switch_modifier, switch_modifier_is_held};
    use config_lib::Modifier;
    use relm4::gtk::gdk::{Key, ModifierType};

    #[test]
    fn releasing_super_while_shift_is_held_ends_switching() {
        let reverse = ModifierType::SUPER_MASK | ModifierType::SHIFT_MASK;
        assert!(switch_modifier_is_held(Modifier::Super, reverse));
        assert!(switch_modifier_is_held(
            Modifier::Super,
            ModifierType::SUPER_MASK
        ));
        assert!(!switch_modifier_is_held(
            Modifier::Super,
            ModifierType::SHIFT_MASK
        ));
        assert!(!switch_modifier_is_held(
            Modifier::Super,
            ModifierType::empty()
        ));
        assert!(switch_modifier_is_held(
            Modifier::None,
            ModifierType::empty()
        ));
    }

    #[test]
    fn only_configured_modifier_releases_close_switch() {
        let modifiers = [
            (Modifier::Alt, [Key::Alt_L, Key::Alt_R]),
            (Modifier::Ctrl, [Key::Control_L, Key::Control_R]),
            (Modifier::Super, [Key::Super_L, Key::Super_R]),
        ];
        for configured in [
            Modifier::Alt,
            Modifier::Ctrl,
            Modifier::Super,
            Modifier::None,
        ] {
            for (modifier, keys) in modifiers {
                for key in keys {
                    assert_eq!(is_switch_modifier(key, configured), modifier == configured);
                }
            }
            for key in [Key::Tab, Key::Shift_L, Key::Shift_R, Key::Escape, Key::a] {
                assert!(!is_switch_modifier(key, configured));
            }
        }
    }
}
