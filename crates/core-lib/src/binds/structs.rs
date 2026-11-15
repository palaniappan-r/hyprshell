#[derive(Debug)]
pub struct ExecBind {
    pub mods: Vec<&'static str>,
    pub key: Box<str>,
    pub exec: String,
    /// Native compositor event; avoids spawning a process for switcher keys.
    pub event: Option<String>,
    pub release: bool,
    pub desc: String,
}
