use super::config;
use super::config::Config;
use super::display_servers::DisplayServer;
use super::display_servers::MockDisplayServer;
use super::event_queue::EventQueueItem;
use super::utils::command::Command;
use super::utils::screen::Screen;
use super::utils::window::Window;
use super::utils::window::WindowHandle;
use super::utils::workspace::Workspace;
use std::collections::VecDeque;

#[derive(Clone)]
pub struct Manager<DM: DisplayServer> {
    pub screens: Vec<Screen>,
    pub windows: Vec<Window>,
    pub workspaces: Vec<Workspace>,
    pub tags: Vec<String>, //list of all known tags
    pub ds: DM,
    focused_workspace_history: VecDeque<usize>,
    focused_window_history: VecDeque<WindowHandle>,
    config: Config,
}

impl<DM: DisplayServer> Manager<DM> {
    pub fn new() -> Manager<DM> {
        let config = config::parse_config();
        let mut m = Manager {
            windows: Vec::new(),
            ds: DM::new(&config),
            screens: Vec::new(),
            workspaces: Vec::new(),
            tags: Vec::new(),
            focused_workspace_history: vec![0],
            focused_window_history: vec![],
            config,
        };
        config::apply_config(&mut m);
        m
    }

    fn active_workspace(&mut self) -> Option<&mut Workspace> {
        let index = self.focused_workspace_history[0];
        if index < self.workspaces.len() {
            return Some(&mut self.workspaces[index]);
        }
        None
    }

    fn focused_window(&mut self) -> Option<&mut Window> {
        if self.focused_window_history.len() == 0 {
            return None;
        }
        let handle = self.focused_window_history[0];
        for w in &mut self.windows {
            if handle == w.handle {
                return Some(w);
            }
        }
        None
    }

    pub fn update_windows(&mut self) {
        {
            let all_windows = &mut self.windows;
            let all: Vec<&mut Window> = all_windows.iter_mut().map(|w| w).collect();
            for w in all {
                w.visable = w.tags.is_empty();
            } // if not tagged force it to display
            for ws in &mut self.workspaces {
                let windows: Vec<&mut Window> = all_windows.iter_mut().map(|w| w).collect();
                ws.update_windows(windows);
            }
        }
        let to_update: Vec<&Window> = (&self.windows).iter().map(|w| w).collect();
        self.ds.update_windows(to_update);
    }

    fn on_new_window(&mut self, a_window: Window) {
        //don't add the window if the manager already knows about it
        for w in &self.windows {
            if w.handle == a_window.handle {
                return;
            }
        }
        let mut window = a_window;
        if let Some(ws) = self.active_workspace() {
            window.tags = ws.tags.clone();
        }
        self.windows.push(window);
        self.update_windows();
    }

    fn on_new_screen(&mut self, screen: Screen) {
        let tag_index = self.workspaces.len();
        let mut workspace = Workspace::from_screen(&screen);
        workspace.name = tag_index.to_string();
        let next_tag = self.tags[tag_index].clone();
        workspace.show_tag(next_tag);
        self.workspaces.push(workspace);
        self.screens.push(screen);
    }

    fn on_destroy_window(&mut self, handle: WindowHandle) {
        let index = self.windows.iter().position(|w| w.handle == handle);
        if let Some(i) = index {
            self.windows.remove(i);
        }
        self.update_windows();
    }

    pub fn on_event(&mut self, event: EventQueueItem) {
        match event {
            EventQueueItem::WindowCreate(w) => self.on_new_window(w),
            EventQueueItem::WindowDestroy(window_handle) => self.on_destroy_window(window_handle),
            EventQueueItem::ScreenCreate(s) => self.on_new_screen(s),
            EventQueueItem::FocusedWindow(window_handle) => {
                self.update_focused_window(window_handle)
            }
            EventQueueItem::Command(command, value) => self.on_command(command, value),
        }
    }

    /*
     * set the focused window if we know about the handle
     */
    fn update_focused_window(&mut self, handle: WindowHandle) {
        while self.focused_window_history.len() > 10 {
            self.focused_window_history.pop_back();
        }
        //self.focused_window_handle = None;
        for w in &self.windows {
            if w.handle == handle {
                if let WindowHandle::XlibHandle(xlibh) = &handle {
                    println!("FOCUSED: {}", xlibh);
                }
                self.focused_window_history.push_front(handle);
                return;
            }
        }
    }

    /*
     * change the active workspace to view a given set of tags
     */
    fn goto_tags(&mut self, tags: Vec<&String>) {
        if let Some(workspace) = self.active_workspace() {
            if tags.len() == 1 {
                workspace.show_tag(tags[0].clone());
            }
            self.update_windows();
        }
    }

    /*
     * move the current focused window to a given tag
     */
    fn move_to_tags(&mut self, tags: Vec<&String>) {
        if let Some(window) = self.focused_window() {
            window.clear_tags();
            for s in tags {
                window.tag(s.clone());
            }
            self.update_windows();
        }
    }

    /*
     * route a command to its correct handler
     */
    pub fn on_command(&mut self, command: Command, value: Option<String>) {
        match command {
            Command::Execute => {}
            //CloseWindow => {},
            //SwapWorkspaces => {},
            Command::GotoTag => {
                if let Some(val) = &value {
                    self.goto_tags(vec![val]);
                }
            }
            Command::MoveToTag => {
                if let Some(val) = &value {
                    self.move_to_tags(vec![val]);
                }
            }

            //MovetoWorkspace => {},
            _ => {}
        }
    }
}

#[allow(dead_code)]
fn mock_manager(screen_counts: i32) -> Manager<MockDisplayServer> {
    let mut manager: Manager<MockDisplayServer> = Manager::new();
    for s in manager.ds.create_fake_screens(screen_counts) {
        manager.on_new_screen(s);
    }
    manager
}

#[test]
fn creating_two_screens_should_tag_them_with_first_and_second_tags() {
    let manager = mock_manager(3); //creates two screens
    assert!(manager.workspaces[0].has_tag("1"));
    assert!(manager.workspaces[1].has_tag("2"));
    assert!(manager.workspaces[2].has_tag("3"));
}

#[test]
fn should_default_to_first_screen() {
    let mut manager = mock_manager(4); //creates two screens
    let expected = manager.workspaces[0].clone();
    let actual = manager.active_workspace().unwrap();
    assert!(actual == &expected);
}

#[test]
fn two_workspaces_should_never_view_the_same_tag() {
    let mut manager = mock_manager(4); //creates two screens
    manager.goto_tags(vec![&"4".to_owned()]);
    let wp1 = manager.active_workspace_mut().unwrap();
    assert!(wp1.has_tag("4"));
    let wp4 = manager.workspaces[3].clone();
    assert!(
        !wp4.has_tag("4"),
        "Expected this workspace to nolonger be displaying 4"
    );
}

//#[test]
//fn should_not_be_able_to_display_the_same_tag_twice() {
//    let manager = mock_manager(3); //creates two screens
//}

#[test]
fn adding_a_second_window_should_resize_the_first() {
    let mut manager = mock_manager(1);
    let w1 = Window::new(WindowHandle::MockHandle(1), None);
    let w2 = Window::new(WindowHandle::MockHandle(2), None);
    manager.on_new_window(w1);
    let w = manager.windows[0].width();
    manager.on_new_window(w2);
    assert!(
        manager.windows[0].width() != w,
        "Expected window to resize when other window was added"
    );
}

#[test]
fn removeing_a_window_should_remove_it_from_the_list_of_managed_windows() {
    let mut manager = mock_manager(1);
    let w1 = Window::new(WindowHandle::MockHandle(1), None);
    let w2 = Window::new(WindowHandle::MockHandle(2), None);
    manager.on_new_window(w1);
    manager.on_new_window(w2);
    manager.on_destroy_window(manager.windows[1].handle.clone());
    assert!(
        manager.windows.len() == 1,
        "Expected only one window to remain"
    );
}

#[test]
fn removeing_a_window_should_resize_the_windows_left_in_the_workspace() {
    let mut manager = mock_manager(1);
    let w1 = Window::new(WindowHandle::MockHandle(1), None);
    let w2 = Window::new(WindowHandle::MockHandle(2), None);
    manager.on_new_window(w1);
    manager.on_new_window(w2);
    let w = manager.windows[0].width();
    manager.on_destroy_window(manager.windows[1].handle.clone());
    assert!(
        manager.windows[0].width() != w,
        "Expected window to resize when other window was removed"
    );
}