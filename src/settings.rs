#[derive(Clone, Debug)]
pub struct ViewerSettings {
    pub wheel_navigation: bool,
    pub hold_to_peek: bool,
    pub hover_pan: bool,
    pub auto_hide_toolbar: bool,
}
impl Default for ViewerSettings {
    fn default() -> Self {
        Self {
            wheel_navigation: true,
            hold_to_peek: true,
            hover_pan: true,
            auto_hide_toolbar: true,
        }
    }
}
