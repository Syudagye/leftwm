use super::Config;
use super::DisplayEvent;
use super::DisplayServer;
use crate::models::Screen;

#[derive(Clone)]
pub struct MockDisplayServer {
    pub screens: Vec<Screen>,
}

impl DisplayServer for MockDisplayServer {
    fn new(_: &impl Config) -> Self {
        Self { screens: vec![] }
    }

    //testing a couple mock event
    fn get_next_events(&mut self) -> Vec<DisplayEvent> {
        vec![]
    }

    fn wait_readable(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>> {
        unimplemented!()
    }

    fn flush(&self) {
        unimplemented!()
    }

    fn generate_verify_focus_event(&self) -> Option<DisplayEvent> {
        unimplemented!()
    }

    fn load_config(
        &mut self,
        _config: &impl Config,
        _focused: Option<&Option<crate::models::WindowHandle>>,
        _windows: &[crate::Window],
    ) {
        unimplemented!()
    }

    fn update_windows(&self, _windows: Vec<&crate::Window>) {
        unimplemented!()
    }

    fn update_workspaces(&self, _focused: Option<&crate::Workspace>) {
        unimplemented!()
    }

    fn execute_action(&mut self, _act: crate::DisplayAction) -> Option<DisplayEvent> {
        unimplemented!()
    }
}
