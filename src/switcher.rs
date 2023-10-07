use std::collections::VecDeque;
use std::sync::mpsc::Receiver;

use super::WorkspaceSwitcherEvent;

pub struct AltTabWorkspaceSwitcher {
    evt_rx: Receiver<WorkspaceSwitcherEvent>,
    // Sway IPC connection
    sway_ipc: swayipc::Connection,
    // Workspace IDs in the most to least recently used order
    mru_workspaces: VecDeque<i64>,
    // Count of tab keypresses in a row, zero means the tab sequence is not triggered
    // Always a valid index for mru_workspaces
    tab_count: usize,
}

impl AltTabWorkspaceSwitcher {
    pub fn new(evt_rx: Receiver<WorkspaceSwitcherEvent>) -> Self {
        let sway_ipc =
            swayipc::Connection::new().expect("sway IPC socket should be available for connection");

        log::debug!("Initialized workspace switcher");

        Self {
            evt_rx,
            sway_ipc,
            mru_workspaces: VecDeque::new(),
            tab_count: 0,
        }
    }

    pub fn run(&mut self) {
        log::info!("Starting the workspace switcher...");

        loop {
            let evt = self.evt_rx.recv().expect("can't read from event channel");
            log::debug!("Processing event: {:?}", evt);

            match evt {
                WorkspaceSwitcherEvent::Trigger => {
                    if self.mru_workspaces.is_empty() {
                        continue;
                    }

                    // Switch to the next workspace, wrapping around if currently at the end
                    self.tab_count = (self.tab_count + 1) % self.mru_workspaces.len();
                    self.switch_to_workspace(self.mru_workspaces[self.tab_count]);
                }
                WorkspaceSwitcherEvent::EndMod => {
                    if self.mru_workspaces.is_empty() {
                        continue;
                    }
                    self.end_sequence(self.mru_workspaces[self.tab_count]);
                }
                WorkspaceSwitcherEvent::SwayWsEvent(ws_event) => {
                    self.handle_ws_event(ws_event.as_ref());
                }
            }

            log::debug!("MRU list: {}", self.format_mru_list());
        }
    }

    fn switch_to_workspace(&mut self, id: i64) {
        let tree = self
            .sway_ipc
            .get_tree()
            .expect("can't get container tree via sway IPC");
        let ws_name = Self::workspace_name_by_id(&tree, id)
            .expect("the id should be associated with an existing workspace (MRU list is probably not in sync)");

        log::debug!(
            "Focusing on workspace with id = {}, name = \"{}\"",
            id,
            ws_name
        );

        self.sway_ipc
            .run_command(format!("workspace {}", ws_name))
            .expect("can't switch workspace using sway IPC command")[0]
            // the only command is `workspace`, its result is at index 0
            .as_ref()
            .expect("can't switch workspace using sway IPC command");
    }

    fn workspace_name_by_id(tree: &swayipc::Node, id: i64) -> Option<&str> {
        tree.nodes
            .iter()
            .flat_map(|output| output.nodes.iter())
            .find(|workspace| workspace.id == id)?
            .name
            .as_deref()
    }

    fn end_sequence(&mut self, new_ws_id: i64) {
        if self.tab_count == 0 {
            return;
        }
        self.mru_workspaces.retain(|&id| id != new_ws_id);
        self.mru_workspaces.push_front(new_ws_id);
        self.tab_count = 0;
    }

    // Reduces code nesting
    #[allow(clippy::comparison_chain)]
    fn handle_ws_event(&mut self, ws_event: &swayipc::WorkspaceEvent) {
        // Sway workspace event types:
        // init - add the to the end of the list
        // empty - remove from the list
        // focus - move to the beginning of the list
        // move, rename, urgent, reload - ignore

        // All events we're interested in have `current` workspace field
        if let Some(current_id) = ws_event.current.as_ref().map(|x| x.id) {
            match ws_event.change {
                swayipc::WorkspaceChange::Init => {
                    self.mru_workspaces.push_back(current_id);
                }
                swayipc::WorkspaceChange::Empty => {
                    if let Some(idx) = self.mru_workspaces.iter().position(|&x| x == current_id) {
                        self.mru_workspaces.remove(idx);
                        if idx < self.tab_count {
                            self.tab_count -= 1;
                        } else if idx == self.tab_count {
                            panic!("the currently focused workspace is deleted");
                        }
                    } else {
                        log::warn!("Deleting unlisted workspace");
                    }
                }
                swayipc::WorkspaceChange::Focus => {
                    if self.tab_count == 0 {
                        self.mru_workspaces.retain(|&x| x != current_id);
                        self.mru_workspaces.push_front(current_id);
                    } else if current_id != self.mru_workspaces[self.tab_count] {
                        // Tab sequence is active and the workspace switch isn't
                        // caused by a tab press, stop the sequence
                        self.end_sequence(current_id);
                    }
                }
                _ => {}
            }
        }
    }

    // For debugging purposes
    fn format_mru_list(&mut self) -> String {
        let tree = self
            .sway_ipc
            .get_tree()
            .expect("can't get container tree via sway IPC");
        format!(
            "{:?}",
            self.mru_workspaces
                .iter()
                .map(|&id| Self::workspace_name_by_id(&tree, id))
                .collect::<Vec<_>>()
        )
    }
}
